/// Extracts text surrounded by required start and end phrases.
/// Matching is case-insensitive and ignores punctuation around marker words.
pub fn extract_between_phrases<'a>(
    transcript: &'a str,
    start_phrase: &str,
    end_phrase: &str,
) -> Option<&'a str> {
    let wanted_start: Vec<String> = words(start_phrase)
        .map(|word| word.to_lowercase())
        .collect();
    let wanted_end: Vec<String> = words(end_phrase).map(|word| word.to_lowercase()).collect();
    if wanted_start.is_empty() || wanted_end.is_empty() {
        return None;
    }

    let found = word_spans(transcript);
    if found.len() <= wanted_start.len() + wanted_end.len() {
        return None;
    }
    let starts_correctly = found
        .iter()
        .zip(&wanted_start)
        .all(|((actual, _, _), expected)| actual.to_lowercase() == *expected);
    let end_offset = found.len() - wanted_end.len();
    let ends_correctly = found[end_offset..]
        .iter()
        .zip(&wanted_end)
        .all(|((actual, _, _), expected)| actual.to_lowercase() == *expected);
    if !starts_correctly || !ends_correctly {
        return None;
    }

    let message_start = found[wanted_start.len()].1;
    let message_end = found[end_offset].1;

    let message = transcript[message_start..message_end]
        .trim_start_matches(|ch: char| ch.is_whitespace() || ",.:;!?-–—".contains(ch))
        .trim_end_matches(|ch: char| ch.is_whitespace() || ",:;-–—".contains(ch))
        .trim();
    (!message.is_empty()).then_some(message)
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
            extract_between_phrases("Viper, defend the bridge! Over.", "Viper", "over"),
            Some("defend the bridge!")
        );
    }

    #[test]
    fn tolerates_case_and_whisper_punctuation() {
        assert_eq!(
            extract_between_phrases("[VIPER]: hello -- OVER!", "viper", "over"),
            Some("hello")
        );
    }

    #[test]
    fn rejects_missing_misplaced_or_empty_markers() {
        assert_eq!(extract_between_phrases("hello over", "Viper", "over"), None);
        assert_eq!(
            extract_between_phrases("Viper hello", "Viper", "over"),
            None
        );
        assert_eq!(
            extract_between_phrases("hello Viper there over", "Viper", "over"),
            None
        );
        assert_eq!(
            extract_between_phrases("Viper hello over there", "Viper", "over"),
            None
        );
        assert_eq!(extract_between_phrases("Viper over", "Viper", "over"), None);
    }

    #[test]
    fn supports_multiword_and_unicode_phrases() {
        assert_eq!(
            extract_between_phrases(
                "Équipe chat, avancez, message terminé",
                "équipe chat",
                "message terminé"
            ),
            Some("avancez")
        );
    }
}
