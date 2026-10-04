//! Resume state in SQLite: which chunks of which download are already on disk.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub url: String,
    pub total: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub chunk_size: u64,
}

pub struct Store(Mutex<Connection>);

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS downloads(
               id TEXT PRIMARY KEY, url TEXT NOT NULL, total INTEGER NOT NULL,
               etag TEXT, last_modified TEXT, chunk_size INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS done_chunks(
               id TEXT NOT NULL, idx INTEGER NOT NULL, PRIMARY KEY(id, idx));",
        )?;
        Ok(Store(Mutex::new(conn)))
    }

    pub fn load(&self, id: &str) -> Result<Option<(Record, Vec<usize>)>> {
        let c = self.0.lock().unwrap();
        let rec = c
            .query_row(
                "SELECT id,url,total,etag,last_modified,chunk_size FROM downloads WHERE id=?1",
                [id],
                |r| {
                    Ok(Record {
                        id: r.get(0)?,
                        url: r.get(1)?,
                        total: r.get::<_, i64>(2)? as u64,
                        etag: r.get(3)?,
                        last_modified: r.get(4)?,
                        chunk_size: r.get::<_, i64>(5)? as u64,
                    })
                },
            )
            .optional()?;
        let Some(rec) = rec else { return Ok(None) };
        let mut stmt = c.prepare("SELECT idx FROM done_chunks WHERE id=?1 ORDER BY idx")?;
        let done = stmt.query_map([id], |r| r.get::<_, i64>(0))?.map(|r| r.map(|i| i as usize)).collect::<std::result::Result<_, _>>()?;
        Ok(Some((rec, done)))
    }

    /// Start (or restart) a download: replaces any previous record and its chunks.
    pub fn begin(&self, rec: &Record) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM done_chunks WHERE id=?1", [&rec.id])?;
        c.execute(
            "INSERT OR REPLACE INTO downloads(id,url,total,etag,last_modified,chunk_size) VALUES(?1,?2,?3,?4,?5,?6)",
            params![rec.id, rec.url, rec.total as i64, rec.etag, rec.last_modified, rec.chunk_size as i64],
        )?;
        Ok(())
    }

    pub fn mark_done(&self, id: &str, idx: usize) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT OR IGNORE INTO done_chunks(id,idx) VALUES(?1,?2)", params![id, idx as i64])?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM done_chunks WHERE id=?1", [id])?;
        c.execute("DELETE FROM downloads WHERE id=?1", [id])?;
        Ok(())
    }
}
