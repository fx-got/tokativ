//! The language a piece of text is written in, named the way speech models name languages.

/// The ISO 639-3 codes whatlang gives, with the ISO 639-1 codes Whisper reports for them.
const CODES: &[(&str, &str)] = &[
    ("afr", "af"),
    ("amh", "am"),
    ("ara", "ar"),
    ("aze", "az"),
    ("bel", "be"),
    ("ben", "bn"),
    ("bul", "bg"),
    ("cat", "ca"),
    ("ces", "cs"),
    ("cmn", "zh"),
    ("cym", "cy"),
    ("dan", "da"),
    ("deu", "de"),
    ("ell", "el"),
    ("eng", "en"),
    ("est", "et"),
    ("fin", "fi"),
    ("fra", "fr"),
    ("guj", "gu"),
    ("heb", "he"),
    ("hin", "hi"),
    ("hrv", "hr"),
    ("hun", "hu"),
    ("hye", "hy"),
    ("ind", "id"),
    ("ita", "it"),
    ("jav", "jw"),
    ("jpn", "ja"),
    ("kan", "kn"),
    ("kat", "ka"),
    ("khm", "km"),
    ("kor", "ko"),
    ("lat", "la"),
    ("lav", "lv"),
    ("lit", "lt"),
    ("mal", "ml"),
    ("mar", "mr"),
    ("mkd", "mk"),
    ("mya", "my"),
    ("nep", "ne"),
    ("nld", "nl"),
    ("nob", "no"),
    ("pan", "pa"),
    ("pes", "fa"),
    ("pol", "pl"),
    ("por", "pt"),
    ("ron", "ro"),
    ("rus", "ru"),
    ("sin", "si"),
    ("slk", "sk"),
    ("slv", "sl"),
    ("sna", "sn"),
    ("spa", "es"),
    ("srp", "sr"),
    ("swe", "sv"),
    ("tam", "ta"),
    ("tel", "te"),
    ("tgl", "tl"),
    ("tha", "th"),
    ("tuk", "tk"),
    ("tur", "tr"),
    ("ukr", "uk"),
    ("urd", "ur"),
    ("uzb", "uz"),
    ("vie", "vi"),
    ("yid", "yi"),
];

/// How sure whatlang has to be. Its own reliability test turns down most Russian sentences
/// because Ukrainian and Belarusian sit close by, though it names them right.
const CONFIDENCE: f64 = 0.4;

/// The language `text` is written in as an ISO 639-1 code, or `None` when there is too little
/// of it to tell.
pub(crate) fn of_text(text: &str) -> Option<&'static str> {
    let info = whatlang::detect(text).filter(|info| info.confidence() >= CONFIDENCE)?;
    let code = info.lang().code();
    CODES
        .iter()
        .find(|(three, _)| *three == code)
        .map(|(_, one)| *one)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_languages_apart() {
        assert_eq!(
            of_text("Сегодня я переписал движок, добавил поиск моделей и всё запушил в main."),
            Some("ru")
        );
        assert_eq!(
            of_text("I fixed the warnings yesterday and pushed everything to the main branch."),
            Some("en")
        );
        assert_eq!(
            of_text(
                "Ich habe gestern die Warnungen behoben und alles auf den Hauptzweig geschoben."
            ),
            Some("de")
        );
        assert_eq!(of_text("ok"), None);
        assert_eq!(of_text("привет"), None);
    }
}
