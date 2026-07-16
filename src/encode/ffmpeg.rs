use anyhow::{Context, Result};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

pub struct FfmpegEncoder {
    child: Child,
    stderr_reader: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
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
        let args = build_args(
            output_path,
            input_audio,
            width,
            height,
            fps,
            codec,
            pix_fmt,
            crf,
            bitrate,
        );

        let mut child = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn ffmpeg. Is ffmpeg installed?")?;

        let mut stderr = child.stderr.take().context("FFmpeg stderr not available")?;
        let stderr_reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            stderr.read_to_end(&mut output)?;
            Ok(output)
        });

        log::info!("FFmpeg encoder started: {}x{} @ {}fps, codec={}", width, height, fps, codec);

        Ok(Self {
            child,
            stderr_reader: Some(stderr_reader),
        })
    }

    pub fn write_frame(&mut self, rgba_pixels: &[u8]) -> Result<()> {
        let stdin = self.child.stdin.as_mut().context("FFmpeg stdin not available")?;
        stdin.write_all(rgba_pixels).context("Failed to write frame to ffmpeg")?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        // Close stdin to signal EOF
        drop(self.child.stdin.take());

        let status = self.child.wait().context("Failed to wait for ffmpeg")?;
        let stderr = self
            .stderr_reader
            .take()
            .context("FFmpeg stderr reader not available")?
            .join()
            .map_err(|_| anyhow::anyhow!("FFmpeg stderr reader thread panicked"))?
            .context("Failed to read FFmpeg stderr")?;

        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr);
            anyhow::bail!("FFmpeg exited with error:\n{}", stderr);
        }

        log::info!("FFmpeg encoding complete");
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn build_args(
    output_path: &Path,
    input_audio: &Path,
    width: u32,
    height: u32,
    fps: u32,
    codec: &str,
    pix_fmt: &str,
    crf: u32,
    bitrate: Option<&str>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-y".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "rgba".into(),
        "-video_size".into(),
        format!("{}x{}", width, height).into(),
        "-framerate".into(),
        fps.to_string().into(),
        "-i".into(),
        "pipe:0".into(),
        "-i".into(),
        input_audio.as_os_str().to_owned(),
        "-c:v".into(),
        codec.into(),
        "-pix_fmt".into(),
        pix_fmt.into(),
    ];

    if let Some(br) = bitrate {
        args.extend([OsString::from("-b:v"), OsString::from(br)]);
    } else {
        args.extend([OsString::from("-crf"), OsString::from(crf.to_string())]);
        args.extend([OsString::from("-preset"), OsString::from("medium")]);
    }

    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-shortest".into(),
        output_path.as_os_str().to_owned(),
    ]);

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_progress_logs_and_preserves_paths() {
        let input = Path::new("audio input.wav");
        let output = Path::new("video output.mp4");
        let args = build_args(output, input, 1280, 720, 30, "libx264", "yuv420p", 18, None);

        assert!(args.windows(2).any(|pair| pair == ["-loglevel", "error"]));
        assert!(args.iter().any(|arg| arg == "-nostats"));
        assert!(args.iter().any(|arg| arg == input.as_os_str()));
        assert_eq!(args.last().unwrap(), output.as_os_str());
    }
}
