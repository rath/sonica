use super::cue::SubtitleCue;
use anyhow::{Context, Result};
use std::path::Path;

pub fn read_srt(path: &Path) -> Result<Vec<SubtitleCue>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read subtitle file: {}", path.display()))?;
    parse_srt(&content)
        .with_context(|| format!("Failed to parse subtitle file: {}", path.display()))
}

pub fn write_srt(path: &Path, cues: &[SubtitleCue]) -> Result<()> {
    let content = format_srt(cues);
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write subtitle file: {}", path.display()))
}

fn parse_srt(content: &str) -> Result<Vec<SubtitleCue>> {
    let normalized = content.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();

    // Line-based scan: a cue starts at a timestamp line ("-->") and its text
    // continues until a blank line, another timestamp line, or the end of file.
    // This is more forgiving than the old `split("\n\n")`, whose hand-edited
    // files (where a blank line often carries stray spaces) merged the next
    // cue's index and timing into the previous cue's text.
    let mut cues = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if !lines[index].contains("-->") {
            index += 1;
            continue;
        }

        let timing = lines[index];
        index += 1;

        let mut text_lines: Vec<&str> = Vec::new();
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() || line.contains("-->") {
                break;
            }
            // A lone digit line immediately above the next timestamp line is
            // that cue's SRT index (hand-edited files sometimes lose the blank
            // separator between blocks), not caption text.
            if line.trim().chars().all(|c| c.is_ascii_digit())
                && lines.get(index + 1).is_some_and(|next| next.contains("-->"))
            {
                break;
            }
            text_lines.push(line);
            index += 1;
        }

        match parse_cue(timing, &text_lines) {
            Ok(cue) => cues.push(cue),
            Err(err) => {
                // One malformed block used to fail the whole render; the CLI
                // advertises a transcribe → hand-edit → render workflow, so
                // a typo in one cue would otherwise burn down every render.
                log::warn!("Skipping malformed subtitle block: {err:#}");
            }
        }
    }

    if cues.is_empty() {
        anyhow::bail!("No valid SRT cues found (the file has no complete cue blocks)");
    }

    cues.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));
    Ok(cues)
}

fn parse_cue(timing: &str, text_lines: &[&str]) -> Result<SubtitleCue> {
    let (start_token, end_token) = timing
        .split_once("-->")
        .map(|(start, tail)| (start.trim(), tail))
        .expect("cue detection only fires on \"-->\" lines");
    let end = end_token
        .split_whitespace()
        .next()
        .context("subtitle cue is missing an end timestamp")?;

    let start_time = parse_timestamp(start_token)?;
    let end_time = parse_timestamp(end)?;
    if end_time < start_time {
        anyhow::bail!("subtitle cue ends before it starts: {timing}");
    }

    let text = text_lines
        .iter()
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        anyhow::bail!("subtitle cue at {timing} has no text");
    }

    Ok(SubtitleCue {
        text,
        start_time,
        end_time,
        words: Vec::new(),
    })
}

fn format_srt(cues: &[SubtitleCue]) -> String {
    let mut output = String::new();
    for (index, cue) in cues.iter().enumerate() {
        output.push_str(&(index + 1).to_string());
        output.push('\n');
        output.push_str(&format_timestamp(cue.start_time));
        output.push_str(" --> ");
        output.push_str(&format_timestamp(cue.end_time));
        output.push('\n');
        output.push_str(&cue.text);
        output.push_str("\n\n");
    }
    output
}

fn parse_timestamp(value: &str) -> Result<f32> {
    let normalized = value.replace('.', ",");
    let (clock, millis) = normalized
        .split_once(',')
        .context("SRT timestamp must include milliseconds")?;
    let parts = clock.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        anyhow::bail!("Invalid SRT timestamp: {value}");
    }

    let hours: u64 = parts[0].parse()?;
    let minutes: u64 = parts[1].parse()?;
    let seconds: u64 = parts[2].parse()?;
    let millis: u64 = millis.parse()?;
    if minutes >= 60 || seconds >= 60 || millis >= 1000 {
        anyhow::bail!("Invalid SRT timestamp: {value}");
    }

    Ok((hours * 3600 + minutes * 60 + seconds) as f32 + millis as f32 / 1000.0)
}

fn format_timestamp(seconds: f32) -> String {
    let total_millis = (seconds.max(0.0) as f64 * 1000.0).round() as u64;
    let millis = total_millis % 1000;
    let total_seconds = total_millis / 1000;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(text: &str, start_time: f32, end_time: f32) -> SubtitleCue {
        SubtitleCue {
            text: text.to_string(),
            start_time,
            end_time,
            words: Vec::new(),
        }
    }

    #[test]
    fn parses_crlf_bom_and_multiline_text() {
        let input = "\u{feff}1\r\n00:00:01,250 --> 00:00:03,500\r\n안녕하세요\r\n반갑습니다\r\n";

        let cues = parse_srt(input).unwrap();

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "안녕하세요 반갑습니다");
        assert_eq!(cues[0].start_time, 1.25);
        assert_eq!(cues[0].end_time, 3.5);
        assert!(cues[0].words.is_empty());
    }

    #[test]
    fn formats_and_round_trips_cues() {
        let original = vec![
            cue("암세포는 미토콘드리아가", 0.03, 1.85),
            cue("손상되었기 때문에", 1.85, 3.15),
        ];

        let encoded = format_srt(&original);
        let decoded = parse_srt(&encoded).unwrap();

        assert!(encoded.contains("00:00:00,030 --> 00:00:01,850"));
        assert_eq!(decoded.len(), original.len());
        assert_eq!(decoded[1].text, original[1].text);
        assert!((decoded[1].end_time - original[1].end_time).abs() < 0.001);
    }

    #[test]
    fn rejects_reversed_timestamps() {
        let input = "1\n00:00:03,000 --> 00:00:02,000\nInvalid\n";
        // The reversed-only block is skipped (after a warning), so the file
        // ends up with no valid cues and parsing fails loudly.
        let err = parse_srt(input).unwrap_err();
        assert!(err.to_string().contains("No valid SRT cues"));
    }

    #[test]
    fn skips_malformed_blocks_and_keeps_the_rest() {
        let input = concat!(
            "1\n00:00:03,000 --> 00:00:02,000\nReversed\n\n",
            "2\n00:00:04,000 --> 00:00:05,000\nGood\n"
        );

        let cues = parse_srt(input).unwrap();

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Good");
    }

    #[test]
    fn terminates_blocks_on_whitespace_blank_lines() {
        let input = concat!(
            "1\n00:00:01,000 --> 00:00:02,000\nhello\n",
            "   \n",
            "2\n00:00:03,000 --> 00:00:04,000\nworld\n"
        );

        let cues = parse_srt(input).unwrap();

        assert_eq!(cues.len(), 2, "a whitespace-only separator must end a block: {cues:?}");
        assert_eq!(cues[0].text, "hello");
        assert_eq!(cues[1].text, "world");
    }

    #[test]
    fn recovers_deleted_blank_separator_before_an_index() {
        // Hand-edited SRT sometimes drops the blank separator; the index line
        // above the next cue's timestamp still unambiguously ends the block.
        let input = concat!(
            "1\n00:00:01,000 --> 00:00:02,000\nhello\n",
            "2\n00:00:03,000 --> 00:00:04,000\nworld\n"
        );

        let cues = parse_srt(input).unwrap();

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "hello");
        assert_eq!(cues[1].text, "world");
    }

    #[test]
    fn rejects_file_without_any_cues() {
        assert!(parse_srt("freeform notes\nno timestamps here\n").is_err());
    }
}
