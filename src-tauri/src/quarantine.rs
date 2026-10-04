//! Mark finished downloads as "downloaded from the internet", as browsers do, so macOS Gatekeeper
//! checks apps and installers before they first open.

use std::path::Path;

#[cfg(any(target_os = "macos", test))]
/// Value for the `com.apple.quarantine` attribute: flags, hex timestamp, and the app that wrote it.
pub fn value(now_secs: u64) -> String {
    format!("0081;{now_secs:x};grabnr;")
}

#[cfg(target_os = "macos")]
pub fn mark(path: &Path, now_secs: u64) -> std::io::Result<()> {
    let status =
        std::process::Command::new("/usr/bin/xattr").arg("-w").arg("com.apple.quarantine").arg(value(now_secs)).arg(path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("xattr failed"))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn mark(_path: &Path, _now_secs: u64) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_has_the_expected_shape() {
        assert_eq!(value(0x6650_1234), "0081;66501234;grabnr;");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn writes_the_attribute() {
        let p = std::env::temp_dir().join(format!("grabnr-quarantine-{}", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        mark(&p, 1_700_000_000).unwrap();
        let out = std::process::Command::new("/usr/bin/xattr").arg("-p").arg("com.apple.quarantine").arg(&p).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), value(1_700_000_000));
        let _ = std::fs::remove_file(p);
    }
}
