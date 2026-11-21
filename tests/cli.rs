use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_barrace")
}

/// Copy the fixture week into a fresh temp dir, optionally rewriting video.toml.
fn temp_fixture(video_toml: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("barrace-cli-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy("tests/fixtures/week/data.csv", dir.join("data.csv")).unwrap();
    let toml = match video_toml {
        Some(t) => t.to_string(),
        None => std::fs::read_to_string("tests/fixtures/week/video.toml").unwrap(),
    };
    std::fs::write(dir.join("video.toml"), toml).unwrap();
    dir
}

fn run_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new(bin()).args(args).current_dir(cwd).output().unwrap()
}

/// (width, height, r_frame_rate, duration, has_audio)
fn probe(file: &Path) -> (u32, u32, String, f64, bool) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,r_frame_rate:format=duration", "-of", "default=nw=1"])
        .arg(file)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| s.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
    (
        get("width").parse().unwrap_or(0),
        get("height").parse().unwrap_or(0),
        get("r_frame_rate"),
        get("duration").parse().unwrap_or(0.0),
        s.contains("codec_type=audio"),
    )
}

#[test]
fn render_main_preview() {
    let dir = temp_fixture(None);
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (w, h, fps, dur, _) = probe(&dir.join("out/main.mp4"));
    assert_eq!((w, h), (960, 540));
    assert_eq!(fps, "30/1");
    assert!((dur - 12.5).abs() < 0.1, "duration {dur}");
}

#[test]
fn render_refuses_unreviewed() {
    let dir = temp_fixture(Some("title = \"T\"\nreviewed = false\n"));
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("reviewed"));
}

#[test]
fn missing_ffmpeg_clear_error() {
    let dir = temp_fixture(None);
    let out = Command::new(bin()).args(["render", ".", "--preview"]).current_dir(&dir).env("PATH", "").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ffmpeg not found"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn render_with_music_has_audio() {
    let dir = temp_fixture(None);
    std::fs::create_dir_all(dir.join("music")).unwrap();
    let st = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=5"])
        .arg(dir.join("music/t.mp3"))
        .status()
        .unwrap();
    assert!(st.success());
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (_, _, _, dur, audio) = probe(&dir.join("out/main.mp4"));
    assert!(audio, "no audio stream");
    assert!((dur - 12.5).abs() < 0.15, "duration {dur}");
}

#[test]
fn no_music_is_silent_ok() {
    let dir = temp_fixture(None);
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no music"));
    let (_, _, _, _, audio) = probe(&dir.join("out/main.mp4"));
    assert!(!audio);
}

#[test]
fn render_short_preview() {
    let dir = temp_fixture(Some("title = \"T\"\nreviewed = true\nshort_seconds = 12\n"));
    let out = run_in(&dir, &["render", ".", "--format", "short", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (w, h, _, dur, _) = probe(&dir.join("out/short.mp4"));
    assert_eq!((w, h), (540, 960));
    assert!((dur - 12.0).abs() < 0.1, "duration {dur}");
}

#[test]
fn render_both() {
    let dir = temp_fixture(Some("title = \"T\"\nreviewed = true\nseconds_per_year = 0.5\nshort_seconds = 8\n"));
    let out = run_in(&dir, &["render", ".", "--format", "both", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("out/main.mp4").exists());
    assert!(dir.join("out/short.mp4").exists());
}

#[test]
fn thumb_png_1280x720() {
    let dir = temp_fixture(None);
    let out = run_in(&dir, &["thumb", "."]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(image::image_dimensions(dir.join("out/thumb.png")).unwrap(), (1280, 720));
}

fn sine(path: &Path) {
    let st = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=f=330:d=3"])
        .arg(path)
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
fn short_bars_option_out_of_range() {
    let dir = temp_fixture(None);
    let out = run_in(&dir, &["render", ".", "--format", "short", "--short-bars", "0", "--preview"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("1–20"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn music_flag_overrides() {
    let dir = temp_fixture(None);
    sine(&dir.join("mine.mp3"));
    let out = run_in(&dir, &["render", ".", "--preview", "--music", "mine.mp3"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(probe(&dir.join("out/main.mp4")).4, "no audio");
}

#[test]
fn no_music_flag_silent() {
    let dir = temp_fixture(None);
    std::fs::create_dir_all(dir.join("music")).unwrap();
    sine(&dir.join("music/t.mp3"));
    let out = run_in(&dir, &["render", ".", "--preview", "--no-music"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!probe(&dir.join("out/main.mp4")).4, "should be silent");
}

#[test]
fn music_and_no_music_conflict() {
    let dir = temp_fixture(None);
    let out = run_in(&dir, &["render", ".", "--music", "x.mp3", "--no-music"]);
    assert!(!out.status.success());
}

fn pillar_fixture(video_toml: Option<&str>, csv: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("barrace-pil-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = csv.map(str::to_string).unwrap_or_else(|| std::fs::read_to_string("tests/fixtures/pillars/pillars.csv").unwrap());
    std::fs::write(dir.join("pillars.csv"), csv).unwrap();
    let toml = video_toml.map(str::to_string).unwrap_or_else(|| std::fs::read_to_string("tests/fixtures/pillars/video.toml").unwrap());
    std::fs::write(dir.join("video.toml"), toml).unwrap();
    dir
}

#[test]
fn render_pillars_preview() {
    let dir = pillar_fixture(None, None);
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (w, h, _, dur, _) = probe(&dir.join("out/main.mp4"));
    assert_eq!((w, h), (960, 540));
    assert!((dur - 12.0).abs() < 0.1, "duration {dur}");
}

#[test]
fn render_pillars_short() {
    let toml = "title = \"P\"\nstyle = \"pillars\"\nshort_seconds = 10\nreviewed = true\n";
    let dir = pillar_fixture(Some(toml), None);
    let out = run_in(&dir, &["render", ".", "--format", "short", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (w, h, _, dur, _) = probe(&dir.join("out/short.mp4"));
    assert_eq!((w, h), (540, 960));
    assert!((dur - 10.0).abs() < 0.1, "duration {dur}");
}

#[test]
fn pillars_thumb() {
    let dir = pillar_fixture(None, None);
    let out = run_in(&dir, &["thumb", "."]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(image::image_dimensions(dir.join("out/thumb.png")).unwrap(), (1280, 720));
}

#[test]
fn pillars_single_row_renders() {
    let dir = pillar_fixture(None, Some("label,name,value\n2000,Solo,5\n"));
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn pillars_refuses_unreviewed() {
    let dir = pillar_fixture(Some("title = \"P\"\nstyle = \"pillars\"\nreviewed = false\n"), None);
    let out = run_in(&dir, &["render", ".", "--preview"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("reviewed"));
}
