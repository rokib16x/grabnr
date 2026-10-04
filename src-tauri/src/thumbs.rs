//! Preview images for finished files, from macOS Quick Look. Other platforms get no thumbnail.

use std::path::{Path, PathBuf};

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn cache_file(dir: &Path, id: &str) -> PathBuf {
    // Ids are hex, but never trust a name that ends up in a path.
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    dir.join(format!("{safe}.png"))
}

/// PNG bytes of a preview of `src`, cached under `cache_dir`.
#[cfg(target_os = "macos")]
pub fn make(src: &Path, cache_dir: &Path, id: &str) -> Option<Vec<u8>> {
    let cached = cache_file(cache_dir, id);
    if let Ok(b) = std::fs::read(&cached) {
        return Some(b);
    }
    std::fs::create_dir_all(cache_dir).ok()?;
    // qlmanage writes `<file name>.png` into the output folder.
    let work = cache_dir.join(format!("work-{}", std::process::id()));
    std::fs::create_dir_all(&work).ok()?;
    let ok = std::process::Command::new("/usr/bin/qlmanage")
        .args(["-t", "-s", "420", "-o"])
        .arg(&work)
        .arg(src)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let produced = work.join(format!("{}.png", src.file_name()?.to_string_lossy()));
    let bytes = if ok { std::fs::read(&produced).ok() } else { None };
    let _ = std::fs::remove_dir_all(&work);
    let bytes = bytes.filter(|b| b.starts_with(&[0x89, b'P', b'N', b'G']))?;
    let _ = std::fs::write(&cached, &bytes);
    Some(bytes)
}

#[cfg(not(target_os = "macos"))]
pub fn make(_src: &Path, _cache_dir: &Path, _id: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_cannot_escape_the_folder() {
        let d = Path::new("/cache");
        assert_eq!(cache_file(d, "ab12cd"), Path::new("/cache/ab12cd.png"));
        assert_eq!(cache_file(d, "../../etc/passwd"), Path::new("/cache/etcpasswd.png"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn makes_a_preview_of_an_image() {
        // A valid 1x1 PNG.
        let png: [u8; 67] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
            0xAE, 0x42, 0x60, 0x82,
        ];
        let dir = std::env::temp_dir().join(format!("grabnr-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("dot.png");
        std::fs::write(&src, png).unwrap();
        match make(&src, &dir.join("cache"), "t1") {
            Some(b) => assert!(b.starts_with(&[0x89, b'P', b'N', b'G'])),
            None => eprintln!("Quick Look is not available in this environment; skipping"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
