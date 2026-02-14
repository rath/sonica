use super::cue::SubtitleCue;
use crate::render::text::TextOverlay;

pub struct SubtitleRenderer {
    cues: Vec<SubtitleCue>,
    overlay: TextOverlay,
    max_chars_per_line: usize,
}

impl SubtitleRenderer {
    pub fn new(cues: Vec<SubtitleCue>, overlay: TextOverlay, max_chars_per_line: usize) -> Self {
        Self {
            cues,
            overlay,
            max_chars_per_line,
        }
    }

    /// Render the active subtitle cue onto the pixel buffer at the given time.
    ///
    /// Uses karaoke-style rendering: words already spoken are bright white,
    /// the currently spoken word is partially highlighted based on time progress,
    /// and upcoming words are rendered in dim white.
    pub fn render_frame(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        time: f32,
    ) {
        let Some(cue) = self.find_active_cue(time) else {
            return;
        };

        // If the cue has no per-word timing data, fall back to plain rendering
        if cue.words.is_empty() {
            self.render_plain(pixels, width, height, cue);
            return;
        }

        self.render_karaoke(pixels, width, height, cue, time);
    }

    /// Karaoke-style rendering: dim base layer + bright overlay for spoken words.
    fn render_karaoke(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        cue: &SubtitleCue,
        time: f32,
    ) {
        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;

        // Split cue words into lines by max_chars
        let lines = self.split_words_into_lines(cue);

        let total_text_height = lines.len() as u32 * font_size
            + (lines.len().saturating_sub(1)) as u32 * line_spacing;

        let pad_x = (font_size as f32 * 0.6) as u32;
        let pad_top = (font_size as f32 * 0.3) as u32;
        let pad_bottom = (font_size as f32 * 0.55) as u32;

        // Compute max line width for background box
        let max_line_width = lines
            .iter()
            .map(|words| {
                let line_text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                self.overlay.measure_width(&line_text)
            })
            .max()
            .unwrap_or(0);

        let bg_w = max_line_width + pad_x * 2;
        let bg_h = total_text_height + pad_top + pad_bottom;

        let margin_bottom = (height as f32 * 0.08) as u32;
        let bg_y = height.saturating_sub(margin_bottom + bg_h);
        let bg_x = if bg_w < width { (width - bg_w) / 2 } else { 0 };

        // Draw semi-transparent black background (35% opacity)
        let bg_color = [0u8, 0, 0, 89];
        TextOverlay::fill_rect(pixels, width, height, bg_x, bg_y, bg_w, bg_h, bg_color);

        let dim_color = [255u8, 255, 255, 100];
        let bright_color = [255u8, 255, 255, 255];
        let text_y = bg_y + pad_top;

        for (i, words) in lines.iter().enumerate() {
            let line_text: String = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            let tw = self.overlay.measure_width(&line_text);
            let line_x = if tw < width { (width - tw) / 2 } else { 0 };
            let y = text_y + i as u32 * (font_size + line_spacing);

            // Pass 1: Draw entire line in dim color
            self.overlay.composite(pixels, width, height, &line_text, line_x, y, dim_color);

            // Pass 2: Overdraw spoken portion in bright color
            self.render_karaoke_highlight(pixels, width, height, words, &line_text, line_x, y, time, bright_color);
        }
    }

    /// Render the bright highlight over spoken words in a single line.
    fn render_karaoke_highlight(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        words: &[super::transcribe::TimedWord],
        line_text: &str,
        line_x: u32,
        y: u32,
        time: f32,
        bright_color: [u8; 4],
    ) {
        // Calculate per-word x positions within the line
        let mut word_x_positions: Vec<u32> = Vec::with_capacity(words.len());
        let mut cursor = 0usize;

        for (wi, word) in words.iter().enumerate() {
            // Measure x offset of this word within line_text
            let prefix = &line_text[..cursor];
            let x_offset = self.overlay.measure_width(prefix);
            word_x_positions.push(x_offset);
            cursor += word.text.len();
            if wi + 1 < words.len() {
                cursor += 1; // space between words
            }
        }

        for (wi, word) in words.iter().enumerate() {
            if time < word.start_time {
                // This word hasn't started yet — stop highlighting
                break;
            }

            let word_x = line_x + word_x_positions[wi];
            let word_width = self.overlay.measure_width(&word.text);

            if time >= word.end_time {
                // Word fully spoken — render entirely in bright
                self.overlay.composite(pixels, width, height, &word.text, word_x, y, bright_color);

                // Also highlight the trailing space if not the last word
                if wi + 1 < words.len() {
                    let space_x = word_x + word_width;
                    self.overlay.composite(pixels, width, height, " ", space_x, y, bright_color);
                }
            } else {
                // Word is currently being spoken — partial highlight
                let duration = word.end_time - word.start_time;
                let progress = if duration > 0.0 {
                    ((time - word.start_time) / duration).clamp(0.0, 1.0)
                } else {
                    1.0
                };

                let clip_x = word_x + (word_width as f32 * progress).round() as u32;
                self.overlay.composite_clipped(
                    pixels, width, height,
                    &word.text, word_x, y,
                    bright_color, clip_x,
                );
                // Current word is partially done — no more words to highlight
                break;
            }
        }
    }

    /// Split cue words into lines respecting max_chars_per_line.
    fn split_words_into_lines<'a>(&self, cue: &'a SubtitleCue) -> Vec<Vec<super::transcribe::TimedWord>> {
        let mut lines: Vec<Vec<super::transcribe::TimedWord>> = Vec::new();
        let mut current_line: Vec<super::transcribe::TimedWord> = Vec::new();
        let mut current_len = 0usize;

        for word in &cue.words {
            let word_len = word.text.len();
            let would_be = if current_line.is_empty() {
                word_len
            } else {
                current_len + 1 + word_len
            };

            if !current_line.is_empty() && would_be > self.max_chars_per_line {
                lines.push(std::mem::take(&mut current_line));
                current_len = 0;
            }

            if current_line.is_empty() {
                current_len = word_len;
            } else {
                current_len += 1 + word_len;
            }
            current_line.push(word.clone());
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        lines
    }

    /// Plain subtitle rendering (no karaoke), used as fallback when words are empty.
    fn render_plain(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        cue: &SubtitleCue,
    ) {
        let lines = wrap_text(&cue.text, self.max_chars_per_line);

        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;
        let total_text_height = lines.len() as u32 * font_size
            + (lines.len().saturating_sub(1)) as u32 * line_spacing;

        let pad_x = (font_size as f32 * 0.6) as u32;
        let pad_top = (font_size as f32 * 0.3) as u32;
        let pad_bottom = (font_size as f32 * 0.55) as u32;

        let max_line_width = lines
            .iter()
            .map(|l| self.overlay.measure_width(l))
            .max()
            .unwrap_or(0);

        let bg_w = max_line_width + pad_x * 2;
        let bg_h = total_text_height + pad_top + pad_bottom;

        let margin_bottom = (height as f32 * 0.08) as u32;
        let bg_y = height.saturating_sub(margin_bottom + bg_h);
        let bg_x = if bg_w < width { (width - bg_w) / 2 } else { 0 };

        let bg_color = [0u8, 0, 0, 89];
        TextOverlay::fill_rect(pixels, width, height, bg_x, bg_y, bg_w, bg_h, bg_color);

        let text_color = [255u8, 255, 255, 255];
        let text_y = bg_y + pad_top;

        for (i, line) in lines.iter().enumerate() {
            let tw = self.overlay.measure_width(line);
            let x = if tw < width { (width - tw) / 2 } else { 0 };
            let y = text_y + i as u32 * (font_size + line_spacing);
            self.overlay.composite(pixels, width, height, line, x, y, text_color);
        }
    }

    /// Binary search for the active cue at the given time.
    fn find_active_cue(&self, time: f32) -> Option<&SubtitleCue> {
        let idx = self
            .cues
            .partition_point(|c| c.start_time <= time);

        if idx == 0 {
            return None;
        }

        let cue = &self.cues[idx - 1];
        if time <= cue.end_time {
            Some(cue)
        } else {
            None
        }
    }
}

/// Wrap text into lines that fit within `max_chars`.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if current_line.len() + 1 + word.len() > max_chars {
            lines.push(current_line.clone());
            current_line.clear();
            current_line.push_str(word);
        } else {
            current_line.push(' ');
            current_line.push_str(word);
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitle::transcribe::TimedWord;

    #[test]
    fn wrap_short_text() {
        let lines = wrap_text("Hello world", 42);
        assert_eq!(lines, vec!["Hello world"]);
    }

    #[test]
    fn wrap_long_text() {
        let lines = wrap_text("This is a somewhat longer sentence that should wrap", 20);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!(line.len() <= 25);
        }
    }

    fn make_cue(text: &str, start: f32, end: f32, words: Vec<TimedWord>) -> SubtitleCue {
        SubtitleCue {
            text: text.to_string(),
            start_time: start,
            end_time: end,
            words,
        }
    }

    fn tw(text: &str, start: f32, end: f32) -> TimedWord {
        TimedWord {
            text: text.to_string(),
            start_time: start,
            end_time: end,
        }
    }

    #[test]
    fn find_active_cue_basic() {
        let cues = vec![
            make_cue("Hello", 1.0, 3.0, vec![tw("Hello", 1.0, 3.0)]),
            make_cue("World", 4.0, 6.0, vec![tw("World", 4.0, 6.0)]),
        ];

        let overlay = TextOverlay::new(24.0, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42);

        assert!(renderer.find_active_cue(0.5).is_none());
        assert_eq!(renderer.find_active_cue(2.0).unwrap().text, "Hello");
        assert!(renderer.find_active_cue(3.5).is_none());
        assert_eq!(renderer.find_active_cue(5.0).unwrap().text, "World");
        assert!(renderer.find_active_cue(7.0).is_none());
    }

    #[test]
    fn split_words_into_lines_respects_max_chars() {
        let cue = make_cue(
            "Hello world this is test",
            0.0, 5.0,
            vec![
                tw("Hello", 0.0, 1.0),
                tw("world", 1.0, 2.0),
                tw("this", 2.0, 3.0),
                tw("is", 3.0, 3.5),
                tw("test", 3.5, 5.0),
            ],
        );

        let overlay = TextOverlay::new(24.0, None, None);
        let renderer = SubtitleRenderer::new(vec![], overlay, 12);

        let lines = renderer.split_words_into_lines(&cue);
        assert!(lines.len() >= 2);
        for line in &lines {
            let text: String = line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            assert!(text.len() <= 14); // slight slack for word boundaries
        }
    }
}
