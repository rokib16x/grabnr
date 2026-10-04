//! Verify a finished file against a hash the user typed in or the server published.

use std::io::Read;
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algo: Algo,
    pub hex: String,
}

impl Checksum {
    /// Accepts `sha256:<hex>`, `sha1:<hex>`, `md5:<hex>`, or bare hex (algorithm guessed from its length).
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (hint, hex) = match s.split_once(':') {
            Some((a, h)) => (Some(a.trim().to_ascii_lowercase().replace('-', "")), h.trim()),
            None => (None, s),
        };
        let hex = hex.to_ascii_lowercase();
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Other("the checksum must be hexadecimal".into()));
        }
        let by_len = match hex.len() {
            32 => Algo::Md5,
            40 => Algo::Sha1,
            64 => Algo::Sha256,
            n => return Err(Error::Other(format!("a checksum of {n} characters is not MD5, SHA-1 or SHA-256"))),
        };
        let algo = match hint.as_deref() {
            None => by_len,
            Some("md5") => Algo::Md5,
            Some("sha1") => Algo::Sha1,
            Some("sha256") => Algo::Sha256,
            Some(other) => return Err(Error::Other(format!("unknown checksum type {other}"))),
        };
        if algo != by_len {
            return Err(Error::Other("the checksum length does not match its type".into()));
        }
        Ok(Checksum { algo, hex })
    }

    /// `sha256:<hex>`, the form `parse` accepts back.
    pub fn spec(&self) -> String {
        let a = match self.algo {
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
            Algo::Sha256 => "sha256",
        };
        format!("{a}:{}", self.hex)
    }

    pub fn label(&self) -> &'static str {
        match self.algo {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
        }
    }
}

fn digest<D: Digest>(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn compute(algo: Algo, path: &Path) -> std::io::Result<String> {
    match algo {
        Algo::Md5 => digest::<Md5>(path),
        Algo::Sha1 => digest::<Sha1>(path),
        Algo::Sha256 => digest::<Sha256>(path),
    }
}

/// Hash of in-memory bytes (used by tests and small payloads).
pub fn compute_bytes(algo: Algo, data: &[u8]) -> String {
    fn go<D: Digest>(d: &[u8]) -> String {
        D::digest(d).iter().map(|b| format!("{b:02x}")).collect()
    }
    match algo {
        Algo::Md5 => go::<Md5>(data),
        Algo::Sha1 => go::<Sha1>(data),
        Algo::Sha256 => go::<Sha256>(data),
    }
}

pub fn verify(sum: &Checksum, path: &Path) -> Result<()> {
    let actual = compute(sum.algo, path)?;
    if actual == sum.hex {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch { algo: sum.label(), expected: sum.hex.clone(), actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_guesses_the_algorithm() {
        let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(Checksum::parse(sha).unwrap().algo, Algo::Sha256);
        assert_eq!(Checksum::parse(&format!("SHA-256: {}", sha.to_uppercase())).unwrap().hex, sha);
        assert_eq!(Checksum::parse("d41d8cd98f00b204e9800998ecf8427e").unwrap().algo, Algo::Md5);
        assert!(Checksum::parse("md5:abcd").is_err());
        assert!(Checksum::parse("not hex at all").is_err());
        assert!(Checksum::parse(&format!("md5:{sha}")).is_err());
    }

    #[test]
    fn hashes_a_file() {
        let p = std::env::temp_dir().join(format!("grabnr-sum-{}", std::process::id()));
        std::fs::write(&p, b"").unwrap();
        assert_eq!(compute(Algo::Sha256, &p).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(compute(Algo::Md5, &p).unwrap(), "d41d8cd98f00b204e9800998ecf8427e");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(compute(Algo::Sha1, &p).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let _ = std::fs::remove_file(p);
    }
}
