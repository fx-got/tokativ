//! Explicit spoken emoji names, backed by Unicode/CLDR data bundled in the binary.

use std::sync::LazyLock;

use regex::Regex;

// Match the generator's tokenization, including combining marks and scripts without spaces.
static WORDS: LazyLock<Regex> = LazyLock::new(|| {
    let characters =
        r"\p{Han}\p{Hiragana}\p{Katakana}\p{Hangul}\p{Thai}\p{Lao}\p{Khmer}\p{Myanmar}";
    Regex::new(&format!(
        r"[{characters}]|[[\p{{L}}\p{{N}}\p{{M}}]&&[^{characters}]]+"
    ))
    .expect("emoji word pattern")
});

static MARKERS: LazyLock<Vec<Vec<String>>> = LazyLock::new(|| {
    [
        "emoji",
        "emojis",
        "émoji",
        "émojis",
        "émoticône",
        "emoticono",
        "emotikon",
        "emotka",
        "emotikona",
        "эмодзи",
        "эмоджи",
        "емодзі",
        "эмодзі",
        "إيموجي",
        "ايموجي",
        "ایموجی",
        "אימוג׳י",
        "אימוג'י",
        "אימוגי",
        "इमोजी",
        "ইমোজি",
        "ਇਮੋਜੀ",
        "ஈமோஜி",
        "ఎమోజి",
        "อีโมจิ",
        "絵文字",
        "表情符号",
        "表情符號",
        "表情",
        "이모지",
    ]
    .into_iter()
    .map(|marker| words(marker).into_iter().map(|word| word.key).collect())
    .collect()
});

struct Names {
    data: &'static str,
    max_words: usize,
}

static NAMES: LazyLock<Names> = LazyLock::new(|| {
    let (header, data) = include_str!("../../../assets/emoji/names.tsv")
        .split_once('\n')
        .expect("emoji header");
    let max_words = header
        .split_whitespace()
        .nth(2)
        .expect("emoji word limit")
        .trim_end_matches(';')
        .parse()
        .expect("numeric emoji word limit");
    Names { data, max_words }
});

impl Names {
    // Search the bundled, sorted lines directly. No large index or startup parsing on the UI
    // thread when the first emoji is spoken; UTF-8 characters are never cut at a byte midpoint.
    fn get(&self, key: &str) -> Option<&'static str> {
        let bytes = self.data.as_bytes();
        let (mut low, mut high) = (0, bytes.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let start = bytes[low..middle]
                .iter()
                .rposition(|&byte| byte == b'\n')
                .map_or(low, |offset| low + offset + 1);
            let end = bytes[start..high]
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(high, |offset| start + offset);
            let (name, emoji) = self.data[start..end].split_once('\t')?;
            match name.cmp(key) {
                std::cmp::Ordering::Less => low = end + 1,
                std::cmp::Ordering::Greater => high = start,
                std::cmp::Ordering::Equal => return Some(emoji),
            }
        }
        None
    }
}

struct Word {
    start: usize,
    end: usize,
    key: String,
}

fn words(text: &str) -> Vec<Word> {
    WORDS
        .find_iter(text)
        .map(|word| Word {
            start: word.start(),
            end: word.end(),
            key: word.as_str().to_lowercase().replace('ё', "е"),
        })
        .collect()
}

fn sentence_break(text: &str) -> bool {
    text.contains(['.', '!', '?', '…', ';', '。', '！', '？', '；', '\n', '\r'])
}

/// Replace both "thumbs up emoji" and "emoji thumbs up". Bare names, unknown names,
/// surrounding text and punctuation are retained exactly as dictated.
pub(crate) fn replace(text: &str) -> String {
    let words = words(text);
    let mut markers = vec![0; words.len()];
    for (index, length) in markers.iter_mut().enumerate() {
        *length = MARKERS
            .iter()
            .filter(|marker| {
                index + marker.len() <= words.len()
                    && marker
                        .iter()
                        .zip(&words[index..])
                        .all(|(key, word)| *key == word.key)
                    && words[index..index + marker.len()]
                        .windows(2)
                        .all(|pair| !sentence_break(&text[pair[0].end..pair[1].start]))
            })
            .map(Vec::len)
            .max()
            .unwrap_or(0);
    }
    if markers.iter().all(|length| *length == 0) {
        return text.to_owned();
    }
    let names = &*NAMES;
    let mut candidates = Vec::new();
    for (index, &marker_length) in markers.iter().enumerate() {
        if marker_length == 0 {
            continue;
        }
        let mut left = None;
        let mut right = None;
        let mut key = String::new();
        for start in (index.saturating_sub(names.max_words)..index).rev() {
            if sentence_break(&text[words[start].end..words[start + 1].start]) {
                break;
            }
            if !key.is_empty() {
                key.insert(0, ' ');
            }
            key.insert_str(0, &words[start].key);
            if let Some(emoji) = names.get(key.as_str()) {
                left = Some((index - start, start, index + marker_length, emoji));
            }
        }
        key.clear();
        for end in index + marker_length..words.len().min(index + marker_length + names.max_words) {
            if sentence_break(&text[words[end - 1].end..words[end].start]) {
                break;
            }
            if !key.is_empty() {
                key.push(' ');
            }
            key.push_str(&words[end].key);
            if let Some(emoji) = names.get(key.as_str()) {
                right = Some((end + 1 - index - marker_length, index, end + 1, emoji));
            }
        }
        let prefer_right = right.is_some_and(|right| {
            left.is_none_or(|left| {
                right.0 > left.0
                    || (right.0 == left.0
                        && index > 0
                        && matches!(
                            words[index - 1].key.as_str(),
                            "and" | "the" | "for" | "with" | "this" | "that" | "и" | "или" | "это"
                        ))
            })
        });
        if let Some(candidate) = if prefer_right { right } else { left.or(right) } {
            candidates.push(candidate);
        }
    }
    // A translated name may itself contain a local word for emoji. Resolve complete names
    // first so an internal marker cannot consume a fragment of a longer, explicit command.
    candidates.sort_by_key(|&(length, start, _, _)| (std::cmp::Reverse(length), start));
    let mut selected: Vec<(usize, usize, &'static str)> = Vec::new();
    for (_, start, end, emoji) in candidates {
        if selected.iter().all(|&(a, b, _)| end <= a || start >= b) {
            selected.push((start, end, emoji));
        }
    }
    selected.sort_by_key(|&(start, _, _)| start);
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    for (index, &(start, mut end, emoji)) in selected.iter().enumerate() {
        if end < words.len()
            && markers[end] > 0
            && !sentence_break(&text[words[end - 1].end..words[end].start])
            && selected
                .get(index + 1)
                .is_none_or(|next| next.0 >= end + markers[end])
        {
            end += markers[end];
        }
        output.push_str(&text[copied..words[start].start]);
        output.push_str(emoji);
        copied = words[end - 1].end;
    }
    output.push_str(&text[copied..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_names_work_before_and_after_the_marker() {
        assert_eq!(replace("Send thumbs up emoji, please."), "Send 👍, please.");
        assert_eq!(replace("EMOJI RED HEART!"), "❤️!");
        assert_eq!(replace("thumbs-up emoji emoji woman technologist"), "👍 👩‍💻");
        assert_eq!(replace("emoji thumbs up emoji"), "👍");
    }

    #[test]
    fn russian_names_and_common_spoken_aliases_work() {
        assert_eq!(
            replace("Поставь эмодзи большой палец вверх."),
            "Поставь 👍."
        );
        assert_eq!(
            replace("красное сердце эмодзи и эмодзи смеётся до слёз"),
            "❤️ и 😂"
        );
        assert_eq!(replace("эмодзи флаг России"), "🇷🇺");
    }

    #[test]
    fn translated_names_and_local_markers_work_across_scripts() {
        for text in [
            "Daumen hoch Emoji",
            "émoji pouce vers le haut",
            "pulgar hacia arriba emoji",
            "थंब्स अप इमोजी",
            "إيموجي قبول",
            "絵文字サムズアップ",
            "拇指向上表情",
            "이모지 올린 엄지",
            "อีโมจิชูนิ้วโป้งขึ้น",
            "אימוג׳י אגודל למעלה",
            "ইমোজি ভালো করেছো",
            "великі пальці вгору емодзі",
        ] {
            assert_eq!(replace(text), "👍", "{text}");
        }
        assert_eq!(replace("émoji cœur rouge"), "❤️");
        assert_eq!(replace("发送红心表情给你"), "发送❤️给你");
        assert_eq!(replace("嘴上有符號的表情 emoji"), "🤬");
        assert_eq!(replace("emoji 嘴上有符號的表情"), "🤬");
        assert_eq!(replace("emoji keycap asterisk"), "*️⃣");
    }

    #[test]
    fn longest_name_wins_including_modifiers_and_zwj_sequences() {
        assert_eq!(replace("thumbs up medium skin tone emoji"), "👍🏽");
        assert_eq!(replace("emoji family woman woman girl boy"), "👩‍👩‍👧‍👦");
        assert_eq!(replace("flag United States emoji"), "🇺🇸");
        assert_eq!(replace("emoji keycap 1"), "1️⃣");
    }

    #[test]
    fn ordinary_words_unknown_names_and_sentence_boundaries_survive() {
        for text in [
            "thumbs up and red heart",
            "unknown imaginary emoji",
            "red heart. Emoji",
            "emoji. Red heart",
            "emojify thumbs up",
            "already 👍❤️",
        ] {
            assert_eq!(replace(text), text);
        }
        assert_eq!(replace("thumbs up emoji, red heart emoji!"), "👍, ❤️!");
        assert_eq!(replace("And emoji red heart."), "And ❤️.");
        assert_eq!(replace("и эмодзи большой палец вверх"), "и 👍");
    }

    #[test]
    fn catalogue_covers_every_fully_qualified_emoji_in_unicode_17() {
        let emoji: std::collections::HashSet<_> = NAMES
            .data
            .lines()
            .filter_map(|line| line.split_once('\t').map(|(_, emoji)| emoji))
            .collect();
        assert_eq!(emoji.len(), 3944);

        for line in NAMES.data.lines().step_by(97) {
            let (name, emoji) = line.split_once('\t').unwrap();
            assert_eq!(NAMES.get(name), Some(emoji), "{name}");
        }
        assert_eq!(NAMES.get("unknown imaginary emoji"), None);
    }
}
