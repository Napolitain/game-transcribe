#[derive(Debug, PartialEq, Eq)]
pub struct PhraseDetection<'a> {
    pub start_detected: bool,
    pub end_detected: bool,
    pub message: Option<&'a str>,
}

/// Detects required start and end phrases and extracts the text between them.
/// Matching is case-insensitive and ignores punctuation around marker words.
pub fn detect_phrases<'a>(
    transcript: &'a str,
    start_phrase: &str,
    end_phrase: &str,
) -> PhraseDetection<'a> {
    let wanted_start: Vec<String> = words(start_phrase)
        .map(|word| word.to_lowercase())
        .collect();
    let wanted_end: Vec<String> = words(end_phrase).map(|word| word.to_lowercase()).collect();
    if wanted_start.is_empty() || wanted_end.is_empty() {
        return PhraseDetection {
            start_detected: false,
            end_detected: false,
            message: None,
        };
    }

    let found = word_spans(transcript);
    let start_detected = found.len() >= wanted_start.len()
        && found
            .iter()
            .zip(&wanted_start)
            .all(|((actual, _, _), expected)| actual.to_lowercase() == *expected);
    let end_detected = found.len() >= wanted_end.len()
        && found[found.len().saturating_sub(wanted_end.len())..]
            .iter()
            .zip(&wanted_end)
            .all(|((actual, _, _), expected)| actual.to_lowercase() == *expected);
    let enough_words = found.len() > wanted_start.len() + wanted_end.len();
    if !start_detected || !end_detected || !enough_words {
        return PhraseDetection {
            start_detected,
            end_detected,
            message: None,
        };
    }

    let end_offset = found.len() - wanted_end.len();
    let message_start = found[wanted_start.len()].1;
    let message_end = found[end_offset].1;

    let message = transcript[message_start..message_end]
        .trim_start_matches(|ch: char| ch.is_whitespace() || ",.:;!?-–—".contains(ch))
        .trim_end_matches(|ch: char| ch.is_whitespace() || ",:;-–—".contains(ch))
        .trim();
    PhraseDetection {
        start_detected,
        end_detected,
        message: (!message.is_empty()).then_some(message),
    }
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|word| !word.is_empty())
}

fn word_spans(text: &str) -> Vec<(&str, usize, usize)> {
    let mut found = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        let is_word = ch.is_alphanumeric() || ch == '\'';
        match (start, is_word) {
            (None, true) => start = Some(index),
            (Some(word_start), false) => {
                found.push((&text[word_start..index], word_start, index));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(word_start) = start {
        found.push((&text[word_start..], word_start, text.len()));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_viper_message_between_required_markers() {
        assert_eq!(
            detect_phrases("Viper, defend the bridge! Over.", "Viper", "over"),
            PhraseDetection {
                start_detected: true,
                end_detected: true,
                message: Some("defend the bridge!")
            }
        );
    }

    #[test]
    fn tolerates_case_and_whisper_punctuation() {
        assert_eq!(
            detect_phrases("[VIPER]: hello -- OVER!", "viper", "over").message,
            Some("hello")
        );
    }

    #[test]
    fn rejects_missing_misplaced_or_empty_markers() {
        assert_eq!(
            detect_phrases("hello over", "Viper", "over"),
            PhraseDetection {
                start_detected: false,
                end_detected: true,
                message: None
            }
        );
        assert_eq!(
            detect_phrases("Viper hello", "Viper", "over"),
            PhraseDetection {
                start_detected: true,
                end_detected: false,
                message: None
            }
        );
        assert_eq!(
            detect_phrases("hello Viper there over", "Viper", "over").message,
            None
        );
        assert_eq!(
            detect_phrases("Viper hello over there", "Viper", "over").message,
            None
        );
        assert_eq!(detect_phrases("Viper over", "Viper", "over").message, None);
    }

    #[test]
    fn supports_multiword_and_unicode_phrases() {
        assert_eq!(
            detect_phrases(
                "Équipe chat, avancez, message terminé",
                "équipe chat",
                "message terminé"
            )
            .message,
            Some("avancez")
        );
    }
}
