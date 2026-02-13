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
        bitrate: Option<&str>,
    ) -> Result<Self> {
        let mut args = vec![
            "-y".to_string(),
            "-f".into(), "rawvideo".into(),
            "-pixel_format".into(), "rgba".into(),
            "-video_size".into(), format!("{}x{}", width, height),
            "-framerate".into(), fps.to_string(),
            "-i".into(), "pipe:0".into(),
            "-i".into(), input_audio.to_str().unwrap().to_string(),
            "-c:v".into(), codec.to_string(),
            "-pix_fmt".into(), pix_fmt.to_string(),
        ];

        if let Some(br) = bitrate {
            args.extend(["-b:v".to_string(), br.to_string()]);
        } else {
            args.extend(["-crf".to_string(), crf.to_string()]);
            args.extend(["-preset".to_string(), "medium".to_string()]);
        }

        args.extend([
            "-c:a".into(), "aac".into(),
            "-b:a".into(), "192k".into(),
            "-shortest".into(),
            output_path.to_str().unwrap().to_string(),
        ]);

        let child = Command::new("ffmpeg")
            .args(&args)
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
