use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub struct FfmpegEncoder {
    child: Child,
}

impl FfmpegEncoder {
    pub fn new(
        output_path: &Path,
        input_audio: &Path,
        width: u32,
        height: u32,
        fps: u32,
        codec: &str,
        pix_fmt: &str,
        crf: u32,
    ) -> Result<Self> {
        let child = Command::new("ffmpeg")
            .args([
                "-y",
                // Video input: raw RGBA from stdin
                "-f", "rawvideo",
                "-pixel_format", "rgba",
                "-video_size", &format!("{}x{}", width, height),
                "-framerate", &fps.to_string(),
                "-i", "pipe:0",
                // Audio input
                "-i", input_audio.to_str().unwrap(),
                // Video encoding
                "-c:v", codec,
                "-pix_fmt", pix_fmt,
                "-crf", &crf.to_string(),
                "-preset", "medium",
                // Audio encoding
                "-c:a", "aac",
                "-b:a", "192k",
                // Sync: use shorter stream
                "-shortest",
                // Output
                output_path.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn ffmpeg. Is ffmpeg installed?")?;

        log::info!("FFmpeg encoder started: {}x{} @ {}fps, codec={}", width, height, fps, codec);

        Ok(Self { child })
    }

    pub fn write_frame(&mut self, rgba_pixels: &[u8]) -> Result<()> {
        let stdin = self.child.stdin.as_mut().context("FFmpeg stdin not available")?;
        stdin.write_all(rgba_pixels).context("Failed to write frame to ffmpeg")?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        // Close stdin to signal EOF
        drop(self.child.stdin.take());

        let output = self.child.wait_with_output().context("Failed to wait for ffmpeg")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("FFmpeg exited with error:\n{}", stderr);
        }

        log::info!("FFmpeg encoding complete");
        Ok(())
    }
}
