use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

#[derive(Debug, Clone)]
pub struct AudioSpec {
    pub path: PathBuf,
    pub duration: f64,
}

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Encoder {
    pub fn start(out: &Path, w: u32, h: u32, fps: u32, audio: Option<AudioSpec>) -> Result<Encoder> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"]);
        if let Some(a) = &audio {
            let fade_out = (a.duration - 3.0).max(0.0);
            cmd.args(["-stream_loop", "-1", "-i"]).arg(&a.path).args([
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-af",
                &format!("afade=t=in:d=2,afade=t=out:st={fade_out:.3}:d=3"),
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-t",
                &format!("{:.3}", a.duration),
            ]);
        }
        cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(out)
            .stdin(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!("ffmpeg not found on PATH"),
            Err(e) => return Err(e).context("starting ffmpeg"),
        };
        let stdin = child.stdin.take();
        Ok(Encoder { child, stdin })
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<()> {
        self.stdin.as_mut().unwrap().write_all(rgba).context("ffmpeg stopped accepting frames")
    }

    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        if !status.success() {
            bail!("ffmpeg failed ({status})");
        }
        Ok(())
    }
}
