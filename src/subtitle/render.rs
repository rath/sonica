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

        let lines = wrap_text(&cue.text, self.max_chars_per_line);

        // composite() uses font_size for vertical glyph placement,
        // so use it (not line_height) for layout calculations.
        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;
        let total_text_height = lines.len() as u32 * font_size
            + (lines.len().saturating_sub(1)) as u32 * line_spacing;

        // Padding around text inside the background box.
        // Extra bottom padding accounts for descenders (g, p, y, etc.)
        // which extend below the font_size baseline.
        let pad_x = (font_size as f32 * 0.6) as u32;
        let pad_top = (font_size as f32 * 0.3) as u32;
        let pad_bottom = (font_size as f32 * 0.55) as u32;

        // Compute max line width for background box
        let max_line_width = lines
            .iter()
            .map(|l| self.overlay.measure_width(l))
            .max()
            .unwrap_or(0);

        // Background box dimensions
        let bg_w = max_line_width + pad_x * 2;
        let bg_h = total_text_height + pad_top + pad_bottom;

        // Position: bottom center with margin
        let margin_bottom = (height as f32 * 0.08) as u32;
        let bg_y = height.saturating_sub(margin_bottom + bg_h);
        let bg_x = if bg_w < width { (width - bg_w) / 2 } else { 0 };

        // Draw semi-transparent black background (35% opacity)
        let bg_color = [0u8, 0, 0, 89];
        TextOverlay::fill_rect(pixels, width, height, bg_x, bg_y, bg_w, bg_h, bg_color);

        // Draw text lines centered within the background
        let text_color = [255u8, 255, 255, 255];
        let text_y = bg_y + pad_top;

        for (i, line) in lines.iter().enumerate() {
            let tw = self.overlay.measure_width(line);
            let x = if tw < width {
                (width - tw) / 2
            } else {
                0
            };
            let y = text_y + i as u32 * (font_size + line_spacing);

            self.overlay.composite(pixels, width, height, line, x, y, text_color);
        }
    }

    /// Binary search for the active cue at the given time.
    fn find_active_cue(&self, time: f32) -> Option<&SubtitleCue> {
        // Cues are sorted by start_time. Find the last cue whose start_time <= time.
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
            assert!(line.len() <= 25); // allows some slack for word boundaries
        }
    }

    #[test]
    fn find_active_cue_basic() {
        let cues = vec![
            SubtitleCue {
                text: "Hello".into(),
                start_time: 1.0,
                end_time: 3.0,
            },
            SubtitleCue {
                text: "World".into(),
                start_time: 4.0,
                end_time: 6.0,
            },
        ];

        let overlay = TextOverlay::new(24.0, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42);

        assert!(renderer.find_active_cue(0.5).is_none());
        assert_eq!(renderer.find_active_cue(2.0).unwrap().text, "Hello");
        assert!(renderer.find_active_cue(3.5).is_none());
        assert_eq!(renderer.find_active_cue(5.0).unwrap().text, "World");
        assert!(renderer.find_active_cue(7.0).is_none());
    }
}
