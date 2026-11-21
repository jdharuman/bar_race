use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// Fresh temp week dir containing the given data.csv and a reviewed video.toml.
pub fn temp_week(csv: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "barrace-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("data.csv"), csv).unwrap();
    std::fs::write(dir.join("video.toml"), "title = \"T\"\nreviewed = true\n").unwrap();
    dir
}

pub const HEADER: &str = "name,category,image,year,value,source,estimated\n";
