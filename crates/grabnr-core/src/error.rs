#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("server returned HTTP {0}")]
    Status(u16),
    #[error("the file changed on the server during the download")]
    FileChanged,
    #[error("download cancelled")]
    Cancelled,
    #[error("download failed on every link: {0}")]
    AllFailed(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
