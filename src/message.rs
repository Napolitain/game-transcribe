/// Removes a leading wake phrase while preserving the original message text.
/// Matching is case-insensitive and ignores punctuation around wake words.
pub fn extract_after_wake_phrase<'a>(transcript: &'a str, wake_phrase: &str) -> Option<&'a str> {
    let wanted: Vec<String> = words(wake_phrase).map(|word| word.to_lowercase()).collect();
    if wanted.is_empty() {
        return None;
    }

    let mut found = words_with_end(transcript);
    let mut end = 0;
    for expected in wanted {
        let (actual, word_end) = found.next()?;
        if actual.to_lowercase() != expected {
            return None;
        }
        end = word_end;
    }

    let message = transcript[end..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || ",.:;!?-–—".contains(ch))
        .trim();
    (!message.is_empty()).then_some(message)
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|word| !word.is_empty())
}

fn words_with_end(text: &str) -> impl Iterator<Item = (&str, usize)> {
    let mut start = None;
    text.char_indices()
        .filter_map(move |(index, ch)| {
            let is_word = ch.is_alphanumeric() || ch == '\'';
            match (start, is_word) {
                (None, true) => {
                    start = Some(index);
                    None
                }
                (Some(word_start), false) => {
                    start = None;
                    Some((&text[word_start..index], index))
                }
                _ => None,
            }
        })
        .chain(
            std::iter::once_with(move || start.map(|word_start| (&text[word_start..], text.len())))
                .flatten(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_case_insensitive_wake_phrase() {
        assert_eq!(
            extract_after_wake_phrase("Game Chat, defend the bridge!", "game chat"),
            Some("defend the bridge!")
        );
    }

    #[test]
    fn tolerates_leading_whisper_punctuation() {
        assert_eq!(
            extract_after_wake_phrase("[game] chat: hello", "game chat"),
            Some("hello")
        );
    }

    #[test]
    fn rejects_non_leading_or_empty_message() {
        assert_eq!(
            extract_after_wake_phrase("hello game chat", "game chat"),
            None
        );
        assert_eq!(extract_after_wake_phrase("game chat", "game chat"), None);
    }

    #[test]
    fn supports_unicode_wake_phrase() {
        assert_eq!(
            extract_after_wake_phrase("Équipe chat, avancez", "équipe chat"),
            Some("avancez")
        );
    }
}
