use anyhow::{Context, Result};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

pub struct FfmpegEncoder {
    child: Option<Child>,
    stderr_reader: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    /// Exact RGBA size of a frame, so a wrong-sized buffer fails loudly
    /// instead of silently corrupting every frame after it.
    frame_bytes: usize,
}

impl FfmpegEncoder {
    #[allow(clippy::too_many_arguments)]
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
            child: Some(child),
            stderr_reader: Some(stderr_reader),
            frame_bytes: (width as usize) * (height as usize) * 4,
        })
    }

    /// Kill the child (if it somehow outlived the error) and return the
    /// diagnostic tail from its stderr, so a mid-render death names the cause
    /// instead of surfacing later as a bare "broken pipe".
    fn take_diagnostics(&mut self) -> String {
        let _ = self.child.as_mut().map(|c| c.kill());
        let _ = self.child.take().map(|mut c| c.wait());
        let stderr = self
            .stderr_reader
            .take()
            .and_then(|handle| handle.join().ok())
            .and_then(|result| result.ok())
            .map(|bytes| {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if text.len() > 4096 {
                    // Keep the tail: ffmpeg's actual error line comes last.
                    text[text.len() - 4096..].to_string()
                } else {
                    text
                }
            })
            .unwrap_or_default();
        if stderr.trim().is_empty() {
            String::new()
        } else {
            format!("\nFFmpeg stderr:\n{}", stderr.trim_end())
        }
    }

    pub fn write_frame(&mut self, rgba_pixels: &[u8]) -> Result<()> {
        if rgba_pixels.len() != self.frame_bytes {
            anyhow::bail!(
                "Frame buffer is {} bytes, but ffmpeg expects {} bytes per {}-pixel RGBA frame",
                rgba_pixels.len(),
                self.frame_bytes,
                self.frame_bytes / 4
            );
        }
        let Some(stdin) = self.child.as_mut().and_then(|c| c.stdin.as_mut()) else {
            anyhow::bail!(
                "ffmpeg encoder is no longer running; the process exited earlier{}",
                self.take_diagnostics()
            );
        };
        if let Err(err) = stdin.write_all(rgba_pixels) {
            anyhow::bail!(
                "Failed to write frame to ffmpeg: {}{}",
                err,
                self.take_diagnostics()
            );
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        // Close stdin to signal EOF
        if let Some(ref mut child) = self.child {
            drop(child.stdin.take());
        }

        let status = self
            .child
            .as_mut()
            .context("ffmpeg encoder is no longer running")?
            .wait()
            .context("Failed to wait for ffmpeg")?;
        self.child = None;

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

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        // On early error paths (e.g. a GPU failure mid-render) ffmpeg keeps
        // waiting for frame data and the out file would stay truncated with a
        // running child. Kill and reap so nothing is left running and the
        // leftover partial output is unambiguous.
        let _ = self.child.as_mut().map(|c| c.kill());
        let _ = self.child.take().map(|mut c| c.wait());
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
