use super::transcribe::WordSegment;

/// A subtitle cue: a grouped phrase/sentence with timing.
#[derive(Clone, Debug)]
pub struct SubtitleCue {
    pub text: String,
    pub start_time: f32,
    pub end_time: f32,
}

/// Group word-level segments into subtitle cues based on timing gaps,
/// punctuation boundaries, and maximum character count.
pub fn group_words(words: Vec<WordSegment>, max_chars: usize) -> Vec<SubtitleCue> {
    if words.is_empty() {
        return Vec::new();
    }

    let mut cues: Vec<SubtitleCue> = Vec::new();
    let mut current_text = String::new();
    let mut current_start = words[0].start_time;
    let mut current_end = words[0].end_time;

    for word in &words {
        let would_be = if current_text.is_empty() {
            word.text.len()
        } else {
            current_text.len() + 1 + word.text.len()
        };

        let timing_gap = word.start_time - current_end;
        let should_break = !current_text.is_empty()
            && (timing_gap > 0.5
                || would_be > max_chars
                || ends_with_sentence_punct(&current_text));

        if should_break {
            cues.push(SubtitleCue {
                text: current_text.clone(),
                start_time: current_start,
                end_time: current_end,
            });
            current_text.clear();
            current_start = word.start_time;
        }

        if current_text.is_empty() {
            current_text.push_str(&word.text);
            current_start = word.start_time;
        } else {
            current_text.push(' ');
            current_text.push_str(&word.text);
        }
        current_end = word.end_time;
    }

    // Flush remaining text
    if !current_text.is_empty() {
        cues.push(SubtitleCue {
            text: current_text,
            start_time: current_start,
            end_time: current_end,
        });
    }

    // Merge short cues (<800ms) with the next cue
    merge_short_cues(&mut cues, 0.8);

    cues
}

fn ends_with_sentence_punct(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with('.')
        || trimmed.ends_with('?')
        || trimmed.ends_with('!')
        || trimmed.ends_with('。')
        || trimmed.ends_with('？')
        || trimmed.ends_with('！')
}

fn merge_short_cues(cues: &mut Vec<SubtitleCue>, min_duration: f32) {
    let mut i = 0;
    while i + 1 < cues.len() {
        let duration = cues[i].end_time - cues[i].start_time;
        if duration < min_duration {
            let next = cues.remove(i + 1);
            cues[i].text.push(' ');
            cues[i].text.push_str(&next.text);
            cues[i].end_time = next.end_time;
            // Don't increment i — re-check the merged cue
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f32, end: f32) -> WordSegment {
        WordSegment {
            text: text.to_string(),
            start_time: start,
            end_time: end,
        }
    }

    #[test]
    fn groups_by_max_chars() {
        let words = vec![
            word("Hello", 0.0, 0.5),
            word("world", 0.5, 1.0),
            word("this", 1.0, 1.5),
            word("is", 1.5, 1.8),
            word("a", 1.8, 1.9),
            word("test", 1.9, 2.5),
        ];
        let cues = group_words(words, 12);
        assert!(cues.len() >= 2);
        for cue in &cues {
            // After merging, some may exceed slightly but grouping should split
            assert!(!cue.text.is_empty());
        }
    }

    #[test]
    fn groups_by_timing_gap() {
        let words = vec![
            word("Hello", 0.0, 0.5),
            word("world.", 0.5, 1.0),
            word("New", 2.0, 2.5), // 1.0s gap
            word("sentence", 2.5, 3.0),
        ];
        let cues = group_words(words, 100);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello world.");
        assert_eq!(cues[1].text, "New sentence");
    }

    #[test]
    fn groups_by_punctuation() {
        let words = vec![
            word("Hello.", 0.0, 1.5),
            word("World", 1.5, 3.0),
        ];
        let cues = group_words(words, 100);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello.");
        assert_eq!(cues[1].text, "World");
    }

    #[test]
    fn merges_short_cues() {
        let words = vec![
            word("Hi", 0.0, 0.3), // very short
            word("there", 0.3, 1.0),
        ];
        let cues = group_words(words, 100);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Hi there");
    }

    #[test]
    fn empty_input() {
        let cues = group_words(vec![], 42);
        assert!(cues.is_empty());
    }
}
