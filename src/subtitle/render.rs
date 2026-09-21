use super::cue::{character_count, SubtitleCue};
use crate::render::text::TextOverlay;
use anyhow::{Context, Result};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug)]
pub struct SubtitleStyle {
    background_color: [u8; 4],
    text_color: [u8; 4],
    dim_color: [u8; 4],
    highlight_color: [u8; 4],
    outline_color: [u8; 4],
    outline_width: u32,
    margin_bottom: f32,
    karaoke: bool,
}

impl SubtitleStyle {
    #[allow(clippy::too_many_arguments)]
    pub fn from_options(
        background_opacity: f32,
        dim_opacity: f32,
        text_color: &str,
        highlight_color: &str,
        outline_color: &str,
        outline_width: u32,
        margin_bottom: f32,
        karaoke: bool,
    ) -> Result<Self> {
        validate_fraction("subtitle background opacity", background_opacity, 1.0)?;
        validate_fraction("subtitle dim opacity", dim_opacity, 1.0)?;
        validate_fraction("subtitle bottom margin", margin_bottom, 0.5)?;

        let text_color = parse_rgb(text_color).context("Invalid subtitle text color")?;
        Ok(Self {
            background_color: with_alpha([0, 0, 0], background_opacity),
            text_color: with_alpha(text_color, 1.0),
            dim_color: with_alpha(text_color, dim_opacity),
            highlight_color: with_alpha(
                parse_rgb(highlight_color).context("Invalid subtitle highlight color")?,
                1.0,
            ),
            outline_color: with_alpha(
                parse_rgb(outline_color).context("Invalid subtitle outline color")?,
                1.0,
            ),
            outline_width,
            margin_bottom,
            karaoke,
        })
    }
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self::from_options(0.55, 0.75, "#FFFFFF", "#FFFFFF", "#000000", 2, 0.08, true)
            .expect("default subtitle style is valid")
    }
}

pub struct SubtitleRenderer {
    cues: Vec<SubtitleCue>,
    overlay: TextOverlay,
    max_chars_per_line: usize,
    style: SubtitleStyle,
    /// Layout is resolution-independent (fonts, wrapping, line widths), so
    /// each cue is laid out once on first activation and re-used every frame
    /// it is on screen. The per-frame render loop previously re-split,
    /// re-joined, and re-measured the same strings dozens of times per frame.
    layouts: RefCell<HashMap<usize, CachedCueLayout>>,
    /// Running maximum of cue end times, so "no cue is active" resolves with
    /// a single binary search instead of scanning backwards over every cue
    /// that started earlier.
    max_end_prefix: Vec<f32>,
}

impl SubtitleRenderer {
    pub fn new(
        cues: Vec<SubtitleCue>,
        overlay: TextOverlay,
        max_chars_per_line: usize,
        style: SubtitleStyle,
    ) -> Self {
        // Cues must be sorted by start_time for the binary searches below
        // (generation sorts; a user SRT is re-sorted by read_srt).
        let mut cues = cues;
        cues.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));

        let mut current_max = f32::MIN;
        let max_end_prefix = cues
            .iter()
            .map(|cue| {
                current_max = current_max.max(cue.end_time);
                current_max
            })
            .collect();

        Self {
            cues,
            overlay,
            max_chars_per_line,
            style,
            layouts: RefCell::new(HashMap::new()),
            max_end_prefix,
        }
    }
}

/// One laid-out subtitle line: joined text plus cached geometry.
struct CachedLine {
    text: String,
    width: u32,
    /// Karaoke-only: the words in reading order.
    words: Vec<super::transcribe::TimedWord>,
    /// Karaoke-only: x offset of each word within the line.
    word_x: Vec<u32>,
    /// Karaoke-only: measured width of each word.
    word_w: Vec<u32>,
}

/// Resolution-independent layout of one cue, built once and reused for every
/// frame the cue is on screen.
struct CachedCueLayout {
    lines: Vec<CachedLine>,
    bg_w: u32,
    bg_h: u32,
}

impl SubtitleRenderer {
    /// Render the active subtitle cue onto the pixel buffer at the given time.
    ///
    /// Uses karaoke-style rendering when per-word timing exists and the style
    /// enables it; otherwise plain rendering.
    pub fn render_frame(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        time: f32,
    ) {
        let Some(cue_idx) = self.find_active_cue_index(time) else {
            return;
        };

        let karaoke = !self.cues[cue_idx].words.is_empty() && self.style.karaoke;

        let layout = self.layout_for(cue_idx);

        if karaoke {
            self.render_karaoke(pixels, width, height, &layout, time);
        } else {
            self.render_plain(pixels, width, height, &layout);
        }
    }

    /// Lazily build and memoize the layout for a cue (first time it shows).
    fn layout_for(&self, cue_idx: usize) -> std::cell::Ref<'_, CachedCueLayout> {
        {
            let mut layouts = self.layouts.borrow_mut();
            layouts
                .entry(cue_idx)
                .or_insert_with(|| self.build_layout(cue_idx));
        }
        std::cell::Ref::map(self.layouts.borrow(), |map| {
            map.get(&cue_idx).expect("layout inserted above")
        })
    }

    fn build_layout(&self, cue_idx: usize) -> CachedCueLayout {
        let cue = &self.cues[cue_idx];
        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;
        let pad_x = (font_size as f32 * 0.6) as u32;
        let pad_top = (font_size as f32 * 0.3) as u32;
        let pad_bottom = (font_size as f32 * 0.55) as u32;

        // Karaoke path: split the timed words into lines; otherwise wrap the
        // plain text. Both produce joined lines with cached widths.
        let use_words = !cue.words.is_empty() && self.style.karaoke;

        let mut lines: Vec<CachedLine> = Vec::new();
        if use_words {
            for line_words in self.split_words_into_lines(cue) {
                let text = line_words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");

                // x offsets accumulate: each word starts where the previous
                // one ended plus one space. measure_width is the sum of the
                // per-character advances, so this matches measuring the whole
                // prefix — without the O(n²) prefix re-measures.
                let mut word_x: Vec<u32> = Vec::with_capacity(line_words.len());
                let mut word_w: Vec<u32> = Vec::with_capacity(line_words.len());
                let mut cursor = 0u32;
                for word in &line_words {
                    let width = self.overlay.measure_width(&word.text);
                    word_x.push(cursor);
                    cursor += width;
                    word_w.push(width);
                    cursor += self.overlay.measure_width(" ");
                }

                lines.push(CachedLine {
                    width: self.overlay.measure_width(&text),
                    text,
                    words: line_words,
                    word_x,
                    word_w,
                });
            }
        } else {
            for text in wrap_text(&cue.text, self.max_chars_per_line) {
                let width = self.overlay.measure_width(&text);
                lines.push(CachedLine {
                    text,
                    width,
                    words: Vec::new(),
                    word_x: Vec::new(),
                    word_w: Vec::new(),
                });
            }
        }

        let total_text_height = lines.len() as u32 * font_size
            + lines.len().saturating_sub(1) as u32 * line_spacing;
        let max_line_width = lines.iter().map(|line| line.width).max().unwrap_or(0);

        CachedCueLayout {
            bg_w: max_line_width + pad_x * 2,
            bg_h: total_text_height + pad_top + pad_bottom,
            lines,
        }
    }

    /// Karaoke-style rendering: dim base layer + bright overlay for spoken words.
    fn render_karaoke(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        layout: &CachedCueLayout,
        time: f32,
    ) {
        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;

        let pad_top = (font_size as f32 * 0.3) as u32;
        let pad_bottom = (font_size as f32 * 0.55) as u32;

        let margin_bottom = (height as f32 * self.style.margin_bottom) as u32;
        let bg_y = height.saturating_sub(margin_bottom + layout.bg_h);
        let bg_x = if layout.bg_w < width { (width - layout.bg_w) / 2 } else { 0 };

        TextOverlay::fill_rect(
            pixels,
            width,
            height,
            bg_x,
            bg_y,
            layout.bg_w,
            layout.bg_h,
            self.style.background_color,
        );
        let text_y = bg_y + pad_top;

        for (i, line) in layout.lines.iter().enumerate() {
            let line_y = text_y + i as u32 * (font_size + line_spacing);
            let line_x = if line.width < width { (width - line.width) / 2 } else { 0 };

            // Pass 1: Draw entire line in dim color
            self.overlay.composite_outlined(
                pixels,
                width,
                height,
                &line.text,
                line_x,
                line_y,
                self.style.dim_color,
                self.style.outline_color,
                self.style.outline_width,
            );

            // Pass 2: Overdraw spoken portion in bright color
            self.render_karaoke_highlight(
                pixels,
                width,
                height,
                line,
                line_x,
                line_y,
                time,
                self.style.highlight_color,
            );
        }
        let _ = pad_bottom; // bottom padding lives inside layout.bg_h
    }

    /// Render the bright highlight over spoken words in a single line,
    /// driven by the line's cached word positions.
    #[allow(clippy::too_many_arguments)]
    fn render_karaoke_highlight(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        line: &CachedLine,
        line_x: u32,
        y: u32,
        time: f32,
        bright_color: [u8; 4],
    ) {
        for (wi, word) in line.words.iter().enumerate() {
            if time < word.start_time {
                // This word hasn't started yet — stop highlighting
                break;
            }

            let word_x = line_x + line.word_x[wi];
            let word_width = line.word_w[wi];

            if time >= word.end_time {
                // Word fully spoken — render entirely in bright
                self.overlay.composite(pixels, width, height, &word.text, word_x, y, bright_color);

                // Also highlight the trailing space if not the last word
                if wi + 1 < line.words.len() {
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
    fn split_words_into_lines(&self, cue: &SubtitleCue) -> Vec<Vec<super::transcribe::TimedWord>> {
        let mut lines: Vec<Vec<super::transcribe::TimedWord>> = Vec::new();
        let mut current_line: Vec<super::transcribe::TimedWord> = Vec::new();
        let mut current_len = 0usize;

        for word in &cue.words {
            let word_len = character_count(&word.text);
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
        layout: &CachedCueLayout,
    ) {
        let font_size = self.overlay.font_size() as u32;
        let line_spacing = (font_size as f32 * 0.2) as u32;
        let pad_top = (font_size as f32 * 0.3) as u32;

        let margin_bottom = (height as f32 * self.style.margin_bottom) as u32;
        let bg_y = height.saturating_sub(margin_bottom + layout.bg_h);
        let bg_x = if layout.bg_w < width { (width - layout.bg_w) / 2 } else { 0 };

        TextOverlay::fill_rect(
            pixels,
            width,
            height,
            bg_x,
            bg_y,
            layout.bg_w,
            layout.bg_h,
            self.style.background_color,
        );
        let text_y = bg_y + pad_top;

        for (i, line) in layout.lines.iter().enumerate() {
            let x = if line.width < width { (width - line.width) / 2 } else { 0 };
            let y = text_y + i as u32 * (font_size + line_spacing);
            self.overlay.composite_outlined(
                pixels,
                width,
                height,
                &line.text,
                x,
                y,
                self.style.text_color,
                self.style.outline_color,
                self.style.outline_width,
            );
        }
    }

    /// Index of the latest cue that is active at `time`.
    fn find_active_cue_index(&self, time: f32) -> Option<usize> {
        let idx = self
            .cues
            .partition_point(|c| c.start_time <= time);

        if idx == 0 {
            return None;
        }

        // The max of end times over the prefix is monotonically non-decreasing,
        // so one comparison decides whether ANY earlier-started cue could still
        // be running; without it, a gap in speech would walk the whole history.
        if self.max_end_prefix[idx - 1] < time {
            return None;
        }

        // Overlapping user SRT cues can outlive cues that start later; walk
        // back to the latest one that ends at/after `time`. For standard
        // non-overlapping files this is exactly one step.
        let mut i = idx - 1;
        while self.cues[i].end_time < time {
            i -= 1;
        }
        Some(i)
    }

    /// Reference to the active cue at the given time (tests use this to
    /// exercise the lookup rules directly; the loop uses
    /// [[SubtitleRenderer::render_frame]]).
    #[cfg(test)]
    fn find_active_cue(&self, time: f32) -> Option<&SubtitleCue> {
        self.find_active_cue_index(time).map(|idx| &self.cues[idx])
    }
}

fn validate_fraction(name: &str, value: f32, maximum: f32) -> Result<()> {
    if value.is_finite() && (0.0..=maximum).contains(&value) {
        Ok(())
    } else {
        anyhow::bail!("{name} must be between 0.0 and {maximum}")
    }
}

fn parse_rgb(value: &str) -> Result<[u8; 3]> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("expected #RRGGBB, got '{value}'");
    }

    Ok([
        u8::from_str_radix(&value[0..2], 16)?,
        u8::from_str_radix(&value[2..4], 16)?,
        u8::from_str_radix(&value[4..6], 16)?,
    ])
}

fn with_alpha(color: [u8; 3], opacity: f32) -> [u8; 4] {
    [
        color[0],
        color[1],
        color[2],
        (opacity * 255.0).round() as u8,
    ]
}

/// Wrap text into lines that fit within `max_chars`.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if character_count(text) <= max_chars {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if character_count(&current_line) + 1 + character_count(word) > max_chars {
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
    fn parses_custom_subtitle_style() {
        let style = SubtitleStyle::from_options(
            0.6,
            0.8,
            "#F0F0F0",
            "00FFAA",
            "#101010",
            3,
            0.12,
            false,
        )
        .unwrap();

        assert_eq!(style.background_color, [0, 0, 0, 153]);
        assert_eq!(style.text_color, [240, 240, 240, 255]);
        assert_eq!(style.dim_color, [240, 240, 240, 204]);
        assert_eq!(style.highlight_color, [0, 255, 170, 255]);
        assert_eq!(style.outline_width, 3);
        assert!(!style.karaoke);
    }

    #[test]
    fn rejects_invalid_subtitle_style_values() {
        assert!(SubtitleStyle::from_options(
            1.1,
            0.75,
            "#FFFFFF",
            "#FFFFFF",
            "#000000",
            2,
            0.08,
            true,
        )
        .is_err());
        assert!(parse_rgb("#FFFF").is_err());
        assert!(validate_fraction("margin", 0.6, 0.5).is_err());
    }

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

    #[test]
    fn wrap_korean_text_by_grapheme_count() {
        let lines = wrap_text("암세포는 미토콘드리아가 손상되었기 때문에", 13);

        assert_eq!(
            lines,
            vec!["암세포는 미토콘드리아가", "손상되었기 때문에"]
        );
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

        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42, SubtitleStyle::default());

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

        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(vec![], overlay, 12, SubtitleStyle::default());

        let lines = renderer.split_words_into_lines(&cue);
        assert!(lines.len() >= 2);
        for line in &lines {
            let text: String = line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            assert!(character_count(&text) <= 14); // slight slack for word boundaries
        }
    }

    #[test]
    fn split_korean_words_respects_max_characters() {
        let cue = make_cue(
            "암세포는 미토콘드리아가 손상되었기 때문에",
            0.0,
            4.0,
            vec![
                tw("암세포는", 0.0, 1.0),
                tw("미토콘드리아가", 1.0, 2.0),
                tw("손상되었기", 2.0, 3.0),
                tw("때문에", 3.0, 4.0),
            ],
        );
        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(vec![], overlay, 13, SubtitleStyle::default());

        let lines = renderer.split_words_into_lines(&cue);

        assert_eq!(lines.len(), 2);
        for line in lines {
            let text = line
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(character_count(&text) <= 13);
        }
    }

    #[test]
    fn overlapping_cues_keep_the_latest_active_one() {
        // Both are on screen at t=4.0; the cue that started later wins.
        let cues = vec![
            make_cue("first", 1.0, 5.0, vec![]),
            make_cue("second", 3.0, 6.0, vec![]),
        ];
        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42, SubtitleStyle::default());

        assert_eq!(renderer.find_active_cue(4.0).unwrap().text, "second");
    }

    #[test]
    fn long_gap_after_many_cues_resolves_to_none() {
        let cues: Vec<SubtitleCue> = (0..200)
            .map(|i| make_cue("x", i as f32, i as f32 + 0.5, vec![]))
            .collect();
        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42, SubtitleStyle::default());

        assert!(renderer.find_active_cue(300.0).is_none());
    }

    #[test]
    fn overlapping_cue_than_ended_yet_still_active_cue_is_found() {
        // Cue 2 (later start) ends at 3.5, but cue 1 still runs until 5.0:
        // at t=4.0 the earlier cue is the only active one.
        let cues = vec![
            make_cue("longer", 1.0, 5.0, vec![]),
            make_cue("short", 3.0, 3.5, vec![]),
        ];
        let overlay = TextOverlay::new(24.0, None, None, None);
        let renderer = SubtitleRenderer::new(cues, overlay, 42, SubtitleStyle::default());

        assert_eq!(renderer.find_active_cue(4.0).unwrap().text, "longer");
    }
}
