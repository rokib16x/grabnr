//! Free-space check, so a big download fails up front instead of at 90%.

use std::path::Path;

use crate::error::{Error, Result};

/// Bytes available to this user on the volume holding `dir`; `None` when it cannot be determined.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

const MARGIN: u64 = 32 << 20;

/// Fail when `need` bytes (plus a small margin) will not fit.
pub fn ensure_space(dir: &Path, need: u64) -> Result<()> {
    match free_space(dir) {
        Some(free) if free < need.saturating_add(MARGIN) => Err(Error::DiskFull { need, free }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_space_and_refuses_the_impossible() {
        let d = std::env::temp_dir();
        #[cfg(unix)]
        assert!(free_space(&d).unwrap() > 0);
        assert!(ensure_space(&d, 1 << 20).is_ok());
        #[cfg(unix)]
        assert!(matches!(ensure_space(&d, u64::MAX / 2), Err(Error::DiskFull { .. })));
    }
}
