use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// File opened once and written at explicit offsets by many workers, no locking needed.
pub struct OffsetFile(File);

impl OffsetFile {
    pub fn open_staging(path: &Path, len: u64) -> io::Result<Self> {
        let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        if f.metadata()?.len() != len {
            f.set_len(len)?;
        }
        Ok(OffsetFile(f))
    }

    pub fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::write_all_at(&self.0, buf, offset)
        }
        #[cfg(windows)]
        {
            let (mut done, mut off) = (0usize, offset);
            while done < buf.len() {
                let n = std::os::windows::fs::FileExt::seek_write(&self.0, &buf[done..], off)?;
                done += n;
                off += n as u64;
            }
            Ok(())
        }
    }

    pub fn sync(&self) -> io::Result<()> {
        self.0.sync_data()
    }
}
