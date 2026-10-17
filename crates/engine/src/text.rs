//! Turns each segment's raw transcript into the exact text to insert after what came before:
//! the user's writing style first, then the spacing and capitals that join it to the text around.

use std::sync::LazyLock;

use regex::Regex;
use tokativ_config::{Capitals, Style};

const TERMINALS: [char; 4] = ['.', '!', '?', '…'];

/// Laughter written with hyphens, in Russian and English: "ха-ха-ха", "хе-хе", "ha-ha".
static LAUGHTER: LazyLock<Regex> = LazyLock::new(|| {
    let syllable = "(?:ха|хе|хи|хо|ha|he|hi|ho)";
    Regex::new(&format!(r"(?i)\b{syllable}(?:-{syllable})+\b")).expect("laughter pattern")
});

/// Rewrites one phrase in the user's style. It runs before [`Joiner::join`], which then sees
/// the phrase as it will be inserted.
pub fn restyle(text: &str, style: &Style) -> String {
    let mut text = text.trim().to_owned();
    for replacement in &style.replacements {
        if replacement.from.is_empty() {
            continue;
        }
        let pattern = format!("(?i){}", regex::escape(&replacement.from));
        if let Ok(pattern) = Regex::new(&pattern) {
            text = pattern
                .replace_all(&text, regex::NoExpand(&replacement.to))
                .into_owned();
        }
    }
    if style.join_laughter {
        text = LAUGHTER
            .replace_all(&text, |found: &regex::Captures| found[0].replace('-', ""))
            .into_owned();
    }
    if !style.final_period && text.ends_with('.') && !text.ends_with("..") {
        text.pop();
    }
    match style.capitals {
        Capitals::AsSpoken => text,
        Capitals::Lowercase => text.to_lowercase(),
        Capitals::LowerStarts => lower_starts(&text),
    }
}

/// Lowercases the first letter of every sentence, sparing words whose capital belongs to them.
fn lower_starts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_start = true;
    for (index, c) in text.char_indices() {
        if at_start && c.is_alphabetic() {
            at_start = false;
            match keeps_capital(&text[index..]) {
                true => out.push(c),
                false => out.extend(c.to_lowercase()),
            }
            continue;
        }
        if TERMINALS.contains(&c) {
            at_start = true;
        }
        out.push(c);
    }
    out
}

/// Joins a session's segments into running text. It remembers whether the text so far ends
/// mid-sentence, so the next segment gets a separating space and a capital only where one
/// belongs.
#[derive(Clone, Debug)]
pub struct Joiner {
    /// Something sits before the cursor that the next segment must be spaced from.
    after_text: bool,
    /// The text before the cursor ends inside a sentence.
    mid_sentence: bool,
    /// Sentence starts get a capital. Off when the style writes them lowercase.
    capitalize: bool,
}

impl Joiner {
    /// A joiner for a session that starts with `before` left of the cursor, when the insertion
    /// backend can tell. Without it the first segment is treated as a fresh sentence. With
    /// `capitalize` off, sentence starts are left as the style wrote them.
    pub fn new(before: Option<&str>, capitalize: bool) -> Self {
        let mut joiner = Self {
            after_text: false,
            mid_sentence: false,
            capitalize,
        };
        if let Some(before) = before {
            let trimmed = before.trim_end();
            joiner.after_text = !before.is_empty() && trimmed.len() == before.len();
            joiner.mid_sentence = trimmed.chars().next_back().is_some_and(|last| {
                !TERMINALS.contains(&last) && !matches!(last, '\n' | ':' | '"' | '»')
            });
        }
        joiner
    }

    /// What [`Joiner::join`] would give for `raw`, without moving past it. For showing a draft.
    pub fn preview(&self, raw: &str) -> Option<String> {
        self.clone().join(raw)
    }

    /// The text to insert for one segment, or `None` when it transcribed to nothing.
    pub fn join(&mut self, raw: &str) -> Option<String> {
        let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(text.len() + 1);
        if self.after_text && !text.starts_with(|c: char| ",.;:!?…)".contains(c)) {
            out.push(' ');
        }
        let mut chars = text.chars();
        if let Some(first) = chars.next() {
            match (self.mid_sentence, keeps_capital(&text)) {
                (false, _) if !self.capitalize => out.push(first),
                (false, _) => out.extend(first.to_uppercase()),
                (true, false) => out.extend(first.to_lowercase()),
                (true, true) => out.push(first),
            }
            out.push_str(chars.as_str());
        }
        self.after_text = true;
        self.mid_sentence = !text.ends_with(TERMINALS);
        Some(out)
    }
}

/// Whether a segment's first word keeps its capital in the middle of a sentence. The engines
/// capitalize every segment as if it opened a sentence, so a capital only stays for "I" and its
/// contractions and for words with more capitals in them, such as acronyms.
fn keeps_capital(text: &str) -> bool {
    let word: String = text
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '\'' || *c == '’')
        .collect();
    let base = word.split(['\'', '’']).next().unwrap_or_default();
    base == "I" || word.chars().skip(1).any(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_join_into_sentences() {
        let mut joiner = Joiner::new(None, true);
        assert_eq!(
            joiner.join("  hello there ").as_deref(),
            Some("Hello there")
        );
        assert_eq!(
            joiner.join("general kenobi.").as_deref(),
            Some(" general kenobi.")
        );
        assert_eq!(joiner.join("привет мир").as_deref(), Some(" Привет мир"));
        assert_eq!(joiner.join("   "), None);
    }

    #[test]
    fn mid_sentence_capitals_drop_unless_they_belong() {
        let mut joiner = Joiner::new(None, true);
        assert_eq!(joiner.join("And so").as_deref(), Some("And so"));
        assert_eq!(joiner.join("Ask not").as_deref(), Some(" ask not"));
        assert_eq!(joiner.join("I think").as_deref(), Some(" I think"));
        assert_eq!(joiner.join("I'm sure").as_deref(), Some(" I'm sure"));
        assert_eq!(joiner.join("NASA knows.").as_deref(), Some(" NASA knows."));
        assert_eq!(joiner.join("then").as_deref(), Some(" Then"));
    }

    #[test]
    fn style_rewrites_phrases() {
        let style = Style {
            capitals: Capitals::LowerStarts,
            final_period: false,
            join_laughter: true,
            replacements: vec![tokativ_config::Replacement {
                from: "токатив".into(),
                to: "Tokativ".into(),
            }],
        };
        assert_eq!(
            restyle("Ха-ха-ха, Токатив работает. Это NASA.", &style),
            "хахаха, Tokativ работает. это NASA"
        );
        let lowercase = Style {
            capitals: Capitals::Lowercase,
            ..Style::default()
        };
        assert_eq!(restyle("Hello There.", &lowercase), "hello there.");
        let mut joiner = Joiner::new(None, false);
        assert_eq!(joiner.join("hi").as_deref(), Some("hi"));
    }

    #[test]
    fn context_before_the_cursor_counts() {
        let mut joiner = Joiner::new(Some("Dear team,"), true);
        assert_eq!(joiner.join("thanks").as_deref(), Some(" thanks"));
        let mut joiner = Joiner::new(Some("Done. "), true);
        assert_eq!(joiner.join("next").as_deref(), Some("Next"));
        let mut joiner = Joiner::new(Some(""), true);
        assert_eq!(joiner.join("start").as_deref(), Some("Start"));
    }
}
