//! Metalink (v3 `.metalink` and v4 `.meta4`): one file described with its size, hashes and mirrors.

use quick_xml::events::Event as Xml;
use quick_xml::Reader;

use crate::checksum::{Algo, Checksum};
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metalink {
    pub name: String,
    pub size: Option<u64>,
    /// Best hash first (SHA-256, then SHA-1, then MD5).
    pub hashes: Vec<Checksum>,
    /// http(s) mirrors, lowest priority number (best) first.
    pub urls: Vec<String>,
}

impl Metalink {
    pub fn checksum(&self) -> Option<Checksum> {
        self.hashes.first().cloned()
    }
}

/// Parse the first `<file>` of a Metalink document.
pub fn parse(xml: &str) -> Result<Metalink> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut m = Metalink::default();
    let (mut in_file, mut done) = (false, false);
    let mut path: Vec<String> = Vec::new();
    let mut hash_type = String::new();
    let mut prio = 1000u32;
    let mut proto_ok = true;
    let mut urls: Vec<(u32, String)> = Vec::new();

    loop {
        match r.read_event().map_err(|e| Error::Other(format!("not a valid Metalink file: {e}")))? {
            Xml::Start(e) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                let attr = |k: &str| e.attributes().flatten().find(|a| a.key.as_ref() == k.as_bytes()).map(|a| String::from_utf8_lossy(&a.value).into_owned());
                match tag.as_str() {
                    "file" if !done => {
                        in_file = true;
                        m.name = attr("name").unwrap_or_default();
                    }
                    "hash" => hash_type = attr("type").unwrap_or_default().to_lowercase().replace('-', ""),
                    "url" => {
                        // v4 uses `priority` (1 = best); v3 uses `preference` (100 = best) and `type`.
                        prio = attr("priority").and_then(|p| p.parse().ok()).or_else(|| attr("preference").and_then(|p| p.parse::<u32>().ok()).map(|p| 1000 - p.min(1000))).unwrap_or(500);
                        proto_ok = attr("type").map(|t| matches!(t.to_lowercase().as_str(), "http" | "https")).unwrap_or(true);
                    }
                    _ => {}
                }
                path.push(tag);
            }
            Xml::Text(t) if in_file => {
                let text = t.unescape().map(|c| c.into_owned()).unwrap_or_default();
                match path.last().map(String::as_str) {
                    Some("size") => m.size = text.trim().parse().ok(),
                    Some("hash") => {
                        let algo = match hash_type.as_str() {
                            "sha256" => Some(Algo::Sha256),
                            "sha1" => Some(Algo::Sha1),
                            "md5" => Some(Algo::Md5),
                            _ => None,
                        };
                        if let (Some(algo), true) = (algo, text.trim().bytes().all(|b| b.is_ascii_hexdigit())) {
                            m.hashes.push(Checksum { algo, hex: text.trim().to_ascii_lowercase() });
                        }
                    }
                    Some("url") if proto_ok && (text.starts_with("http://") || text.starts_with("https://")) => urls.push((prio, text.trim().to_string())),
                    _ => {}
                }
            }
            Xml::End(e) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                if tag == "file" && in_file {
                    in_file = false;
                    done = true;
                }
                path.pop();
            }
            Xml::Eof => break,
            _ => {}
        }
    }
    if !done {
        return Err(Error::Other("the Metalink file does not describe a file".into()));
    }
    urls.sort_by_key(|(p, _)| *p);
    m.urls = urls.into_iter().map(|(_, u)| u).collect();
    m.urls.dedup();
    if m.urls.is_empty() {
        return Err(Error::Other("the Metalink file lists no http or https mirrors".into()));
    }
    let rank = |c: &Checksum| match c.algo {
        Algo::Sha256 => 0,
        Algo::Sha1 => 1,
        Algo::Md5 => 2,
    };
    m.hashes.sort_by_key(rank);
    Ok(m)
}

/// Does this look like a Metalink document (by file name or content type)?
pub fn looks_like(url_or_name: &str, content_type: Option<&str>) -> bool {
    let u = url_or_name.split(['?', '#']).next().unwrap_or("").to_lowercase();
    u.ends_with(".meta4") || u.ends_with(".metalink") || content_type.map(|c| c.contains("metalink")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V4: &str = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
 <file name="distro.iso">
  <size>4096</size>
  <hash type="md5">d41d8cd98f00b204e9800998ecf8427e</hash>
  <hash type="sha-256">e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855</hash>
  <url priority="2">https://b.example/distro.iso</url>
  <url priority="1">https://a.example/distro.iso</url>
  <url priority="1">ftp://ftp.example/distro.iso</url>
 </file>
</metalink>"#;

    const V3: &str = r#"<metalink version="3.0" xmlns="http://www.metalinker.org/">
 <files><file name="x.zip">
  <size>10</size>
  <verification><hash type="sha1">a9993e364706816aba3e25717850c26c9cd0d89d</hash></verification>
  <resources>
   <url type="http" preference="50">http://slow.example/x.zip</url>
   <url type="http" preference="100">http://fast.example/x.zip</url>
   <url type="ftp" preference="100">ftp://f.example/x.zip</url>
   <url type="bittorrent">http://t.example/x.torrent</url>
  </resources>
 </file></files></metalink>"#;

    #[test]
    fn parses_v4() {
        let m = parse(V4).unwrap();
        assert_eq!(m.name, "distro.iso");
        assert_eq!(m.size, Some(4096));
        assert_eq!(m.urls, vec!["https://a.example/distro.iso", "https://b.example/distro.iso"]);
        assert_eq!(m.checksum().unwrap().algo, Algo::Sha256, "the strongest hash comes first");
    }

    #[test]
    fn parses_v3_and_ranks_by_preference() {
        let m = parse(V3).unwrap();
        assert_eq!(m.urls, vec!["http://fast.example/x.zip", "http://slow.example/x.zip"]);
        assert_eq!(m.checksum().unwrap().hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn rejects_documents_without_usable_mirrors() {
        assert!(parse("<html></html>").is_err());
        assert!(parse(r#"<metalink><file name="a"><url>ftp://x/a</url></file></metalink>"#).is_err());
    }

    #[test]
    fn detects_by_name_or_type() {
        assert!(looks_like("https://x/a.meta4?dl=1", None));
        assert!(looks_like("https://x/get", Some("application/metalink4+xml")));
        assert!(!looks_like("https://x/a.iso", Some("application/octet-stream")));
    }
}
