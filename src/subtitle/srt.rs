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
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let lines = block
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            continue;
        }

        let timing_index = lines
            .iter()
            .position(|line| line.contains("-->"))
            .context("Subtitle cue is missing a timestamp line")?;
        let timing = lines[timing_index];
        let (start, end) = timing
            .split_once("-->")
            .context("Invalid SRT timestamp separator")?;
        let start_time = parse_timestamp(start.trim())?;
        let end_token = end
            .split_whitespace()
            .next()
            .context("Subtitle cue is missing an end timestamp")?;
        let end_time = parse_timestamp(end_token)?;
        if end_time < start_time {
            anyhow::bail!("Subtitle cue ends before it starts: {timing}");
        }

        let text = lines[timing_index + 1..].join(" ");
        if text.is_empty() {
            anyhow::bail!("Subtitle cue at {timing} has no text");
        }

        cues.push(SubtitleCue {
            text,
            start_time,
            end_time,
            words: Vec::new(),
        });
    }

    if cues.is_empty() {
        anyhow::bail!("Subtitle file contains no cues");
    }

    cues.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));
    Ok(cues)
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
        assert!(parse_srt(input).is_err());
    }
}
