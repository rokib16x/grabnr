use reqwest::header::{HeaderMap, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::{Client, StatusCode};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Probe {
    /// URL after redirects; chunk requests go straight here.
    pub final_url: String,
    pub total: Option<u64>,
    pub ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub filename: String,
}

impl Probe {
    /// Value for `If-Range`, so a changed file yields 200 instead of a corrupt splice.
    pub fn validator(&self) -> Option<&str> {
        self.etag.as_deref().or(self.last_modified.as_deref())
    }
}

/// One tiny `Range: bytes=0-0` request tells us size, range support and validators.
pub async fn probe(client: &Client, url: &str, headers: &HeaderMap) -> Result<Probe> {
    let resp = client.get(url).headers(headers.clone()).header(RANGE, "bytes=0-0").send().await?;
    let status = resp.status();
    let h = resp.headers().clone();
    let final_url = resp.url().to_string();
    let text = |name| h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);

    let (ranges, total) = match status {
        StatusCode::PARTIAL_CONTENT => (true, text(CONTENT_RANGE).and_then(|v| parse_total(&v))),
        StatusCode::RANGE_NOT_SATISFIABLE => (true, text(CONTENT_RANGE).and_then(|v| parse_total(&v)).or(Some(0))),
        s if s.is_success() => {
            let _ = text(ACCEPT_RANGES); // 200 to a ranged request means ranges are not honoured
            (false, text(CONTENT_LENGTH).and_then(|v| v.parse().ok()))
        }
        s => return Err(Error::Status(s.as_u16())),
    };
    let filename = text(CONTENT_DISPOSITION).and_then(|v| disposition_filename(&v)).unwrap_or_else(|| filename_from_url(&final_url));
    Ok(Probe { final_url, total, ranges, etag: text(ETAG), last_modified: text(LAST_MODIFIED), filename: sanitize(&filename) })
}

fn parse_total(content_range: &str) -> Option<u64> {
    content_range.rsplit('/').next()?.trim().parse().ok()
}

fn disposition_filename(v: &str) -> Option<String> {
    for part in v.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename*=") {
            let rest = rest.rsplit('\'').next().unwrap_or(rest);
            return Some(percent_decode(rest));
        }
    }
    for part in v.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

fn filename_from_url(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let path = no_query.split_once("://").map(|(_, rest)| rest).unwrap_or(no_query);
    let last = path.split_once('/').map(|(_, p)| p.rsplit('/').next().unwrap_or("")).unwrap_or("");
    if last.is_empty() {
        "download".into()
    } else {
        percent_decode(last)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strip path separators and control characters so a server cannot write outside the target folder.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() || c == ':' { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "download".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(filename_from_url("https://x.com/a/b%20c.zip?t=1"), "b c.zip");
        assert_eq!(filename_from_url("https://x.com/"), "download");
        assert_eq!(disposition_filename("attachment; filename=\"a.bin\""), Some("a.bin".into()));
        assert_eq!(disposition_filename("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"), Some("résumé.pdf".into()));
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
    }

    #[test]
    fn total_from_content_range() {
        assert_eq!(parse_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_total("bytes */0"), Some(0));
        assert_eq!(parse_total("bytes 0-0/*"), None);
    }
}
