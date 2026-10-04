//! Non-HTTP sources (FTP now): one interface the download engine uses to read pieces of a remote file.

use std::time::Duration;

use tokio::io::AsyncRead;

use crate::download::Route;
use crate::error::{Error, Result};
use crate::ftp;
use crate::sftp;

/// Where a remote file is, by protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Ftp(ftp::Target),
    Sftp(sftp::Target),
}

impl Target {
    pub fn path(&self) -> &str {
        match self {
            Target::Ftp(t) => &t.path,
            Target::Sftp(t) => &t.path,
        }
    }

    pub fn host(&self) -> &str {
        match self {
            Target::Ftp(t) => &t.host,
            Target::Sftp(t) => &t.host,
        }
    }
}

/// Is this a link the remote path handles (not plain HTTP)?
pub fn looks_like(url: &str) -> bool {
    ftp::looks_like(url) || sftp::looks_like(url)
}

pub fn parse(url: &str) -> Result<Target> {
    if ftp::looks_like(url) {
        return Ok(Target::Ftp(ftp::parse(url)?));
    }
    if sftp::looks_like(url) {
        return Ok(Target::Sftp(sftp::parse(url)?));
    }
    Err(Error::Other("unsupported link type".into()))
}

/// What a probe of the remote file found.
#[derive(Debug, Clone)]
pub struct Stat {
    pub size: u64,
    /// Modification time as text, to notice a changed file when resuming.
    pub modified: Option<String>,
}

// One value per worker connection, so the size difference between the variants does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Session {
    Ftp(ftp::Session),
    Sftp(sftp::Session),
}

pub type Reader = Box<dyn AsyncRead + Send + Unpin>;

impl Session {
    pub async fn connect(t: &Target, route: &Route, stall: Duration, ssh: &sftp::SshOptions) -> Result<Session> {
        match t {
            Target::Ftp(t) => Ok(Session::Ftp(ftp::Session::connect(t, route, stall).await?)),
            Target::Sftp(t) => Ok(Session::Sftp(sftp::Session::connect(t, route, stall, ssh).await?)),
        }
    }

    /// Size and modification time; fails when the server cannot seek (then pieces cannot be fetched in parallel).
    pub async fn stat(&mut self, t: &Target) -> Result<Stat> {
        match (self, t) {
            (Session::Ftp(s), Target::Ftp(t)) => {
                let size = s.size(&t.path).await?;
                let modified = s.mdtm(&t.path).await;
                if !s.supports_rest().await {
                    return Err(Error::Other(
                        "this FTP server cannot start a download part-way (REST), so it cannot be split over connections".into(),
                    ));
                }
                Ok(Stat { size, modified })
            }
            (Session::Sftp(s), Target::Sftp(t)) => {
                let (size, modified) = s.stat(&t.path).await?;
                Ok(Stat { size, modified })
            }
            _ => Err(Error::Other("mismatched session".into())),
        }
    }

    /// Read the file from `start`; `len` is how many bytes the caller will take. The caller takes what it needs, drops the
    /// reader and calls [`Session::end_transfer`].
    pub async fn open_range(&mut self, t: &Target, start: u64, len: u64) -> Result<Reader> {
        match (self, t) {
            (Session::Ftp(s), Target::Ftp(t)) => Ok(Box::new(s.open_range(&t.path, start).await?)),
            (Session::Sftp(s), Target::Sftp(t)) => Ok(Box::new(s.open_range(&t.path, start, len).await?)),
            _ => Err(Error::Other("mismatched session".into())),
        }
    }

    /// Make the connection ready for the next piece; false means it should be dropped.
    pub async fn end_transfer(&mut self) -> bool {
        match self {
            Session::Ftp(s) => s.end_transfer().await,
            // The file handle closes when the reader is dropped; there is no reply to wait for.
            Session::Sftp(_) => true,
        }
    }
}
