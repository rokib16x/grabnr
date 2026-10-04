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

/// A finished chunk: its index and the CRC-32 of its bytes (none for databases from before checksums were stored).
pub type DoneChunk = (usize, Option<u32>);

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
               id TEXT NOT NULL, idx INTEGER NOT NULL, crc INTEGER, PRIMARY KEY(id, idx));",
        )?;
        // Databases from before checksums were stored: add the column; those chunks have no checksum and are fetched again.
        let has_crc: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('done_chunks') WHERE name='crc'", [], |r| r.get(0))?;
        if has_crc == 0 {
            conn.execute("ALTER TABLE done_chunks ADD COLUMN crc INTEGER", [])?;
        }
        Ok(Store(Mutex::new(conn)))
    }

    /// The record and the finished chunks as `(index, CRC-32 of its bytes)`. A chunk without a checksum cannot be verified.
    pub fn load(&self, id: &str) -> Result<Option<(Record, Vec<DoneChunk>)>> {
        let c = self.0.lock().unwrap();
        let rec = c
            .query_row("SELECT id,url,total,etag,last_modified,chunk_size FROM downloads WHERE id=?1", [id], |r| {
                Ok(Record {
                    id: r.get(0)?,
                    url: r.get(1)?,
                    total: r.get::<_, i64>(2)? as u64,
                    etag: r.get(3)?,
                    last_modified: r.get(4)?,
                    chunk_size: r.get::<_, i64>(5)? as u64,
                })
            })
            .optional()?;
        let Some(rec) = rec else { return Ok(None) };
        let mut stmt = c.prepare("SELECT idx, crc FROM done_chunks WHERE id=?1 ORDER BY idx")?;
        let done = stmt
            .query_map([id], |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, Option<i64>>(1)?.map(|v| v as u32))))?
            .collect::<std::result::Result<_, _>>()?;
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

    pub fn mark_done(&self, id: &str, idx: usize, crc: u32) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("INSERT OR REPLACE INTO done_chunks(id,idx,crc) VALUES(?1,?2,?3)", params![id, idx as i64, crc as i64])?;
        Ok(())
    }

    /// Drop chunks that turned out damaged so they are downloaded again.
    pub fn forget(&self, id: &str, idxs: &[usize]) -> Result<()> {
        let c = self.0.lock().unwrap();
        for i in idxs {
            c.execute("DELETE FROM done_chunks WHERE id=?1 AND idx=?2", params![id, *i as i64])?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM done_chunks WHERE id=?1", [id])?;
        c.execute("DELETE FROM downloads WHERE id=?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Record {
        Record { id: "d".into(), url: "u".into(), total: 10, etag: None, last_modified: None, chunk_size: 5 }
    }

    #[test]
    fn keeps_checksums_and_forgets_chunks() {
        let s = Store::in_memory().unwrap();
        s.begin(&rec()).unwrap();
        s.mark_done("d", 0, 0xDEADBEEF).unwrap();
        s.mark_done("d", 1, 7).unwrap();
        assert_eq!(s.load("d").unwrap().unwrap().1, vec![(0, Some(0xDEADBEEF)), (1, Some(7))]);
        s.forget("d", &[0]).unwrap();
        assert_eq!(s.load("d").unwrap().unwrap().1, vec![(1, Some(7))]);
    }

    #[test]
    fn old_databases_are_upgraded_and_their_chunks_have_no_checksum() {
        let dir = std::env::temp_dir().join(format!("grabnr-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE downloads(id TEXT PRIMARY KEY, url TEXT NOT NULL, total INTEGER NOT NULL, etag TEXT, last_modified TEXT, chunk_size INTEGER NOT NULL);
                 CREATE TABLE done_chunks(id TEXT NOT NULL, idx INTEGER NOT NULL, PRIMARY KEY(id, idx));
                 INSERT INTO downloads VALUES('d','u',10,NULL,NULL,5);
                 INSERT INTO done_chunks VALUES('d',0);",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load("d").unwrap().unwrap().1, vec![(0, None)]);
        s.mark_done("d", 1, 9).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
