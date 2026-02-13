use fontdue::{Font, FontSettings};

const FONT_DATA: &[u8] = include_bytes!("../../assets/LiberationMono-Regular.ttf");

pub struct TextOverlay {
    font: Font,
    font_size: f32,
}

impl TextOverlay {
    pub fn new(font_size: f32) -> Self {
        let font = Font::from_bytes(FONT_DATA, FontSettings::default())
            .expect("Failed to load embedded font");
        Self { font, font_size }
    }

    /// Composite text onto an RGBA pixel buffer at the given position.
    pub fn composite(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        text: &str,
        x: u32,
        y: u32,
        color: [u8; 4],
    ) {
        let mut cursor_x = x as i32;
        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, self.font_size);
            let glyph_y = y as i32 + self.font_size as i32 - metrics.height as i32 - metrics.ymin;

            for gy in 0..metrics.height {
                for gx in 0..metrics.width {
                    let alpha = bitmap[gy * metrics.width + gx];
                    if alpha == 0 {
                        continue;
                    }

                    let px = cursor_x + gx as i32;
                    let py = glyph_y + gy as i32;

                    if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                        continue;
                    }

                    let idx = ((py as u32 * width + px as u32) * 4) as usize;
                    if idx + 3 >= pixels.len() {
                        continue;
                    }

                    let a = alpha as f32 / 255.0 * (color[3] as f32 / 255.0);
                    let inv_a = 1.0 - a;
                    pixels[idx] = (color[0] as f32 * a + pixels[idx] as f32 * inv_a) as u8;
                    pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv_a) as u8;
                    pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv_a) as u8;
                    pixels[idx + 3] = 255;
                }
            }

            cursor_x += metrics.advance_width as i32;
        }
    }

    /// Measure the width of rendered text in pixels.
    pub fn measure_width(&self, text: &str) -> u32 {
        let mut width = 0.0f32;
        for ch in text.chars() {
            let (metrics, _) = self.font.rasterize(ch, self.font_size);
            width += metrics.advance_width;
        }
        width.ceil() as u32
    }
}
