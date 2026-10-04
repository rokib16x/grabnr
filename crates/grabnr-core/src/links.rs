//! Batch URL lists and a simple page link grabber.

use url::Url;

/// One URL per line. Blank lines and `#` comments are skipped, duplicates dropped, and only http(s) is kept.
pub fn parse_url_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Ok(u) = Url::parse(l) {
            if matches!(u.scheme(), "http" | "https") && !out.iter().any(|x| x == u.as_str()) {
                out.push(u.to_string());
            }
        }
    }
    out
}

/// Like [`parse_url_list`] for lines already split into a vector.
pub fn parse_url_list_from_vec(lines: &[String]) -> Vec<String> {
    parse_url_list(&lines.join("\n"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PageLink {
    pub url: String,
    /// Link text, or the file name when the link has none.
    pub text: String,
}

/// Links found in an HTML page (`href` and `src`), resolved against `base`. Fragment-only, mailto, javascript
/// and duplicate links are dropped.
pub fn extract_links(html: &str, base: &str) -> Vec<PageLink> {
    let Ok(base) = Url::parse(base) else { return Vec::new() };
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<PageLink> = Vec::new();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find('<') {
        let start = pos + rel;
        let Some(end_rel) = lower[start..].find('>') else { break };
        let end = start + end_rel;
        let tag = &html[start + 1..end];
        let tag_lc = &lower[start + 1..end];
        let is_a = tag_lc.starts_with("a ") || tag_lc.starts_with("a\n");
        let is_src = tag_lc.starts_with("source ") || tag_lc.starts_with("video ") || tag_lc.starts_with("audio ");
        if is_a || is_src {
            if let Some(href) = attr(tag, tag_lc, if is_a { "href" } else { "src" }) {
                if let Some(url) = resolve(&base, &href) {
                    let text = if is_a { inner_text(html, &lower, end + 1) } else { String::new() };
                    if !out.iter().any(|l| l.url == url) {
                        let text = if text.is_empty() { file_name(&url) } else { text };
                        out.push(PageLink { url, text });
                    }
                }
            }
        }
        pos = end + 1;
    }
    out
}

fn resolve(base: &Url, href: &str) -> Option<String> {
    let h = href.trim();
    if h.is_empty() || h.starts_with('#') {
        return None;
    }
    let mut u = base.join(h).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    u.set_fragment(None);
    Some(u.to_string())
}

fn attr(tag: &str, tag_lc: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let mut from = 0;
    while let Some(i) = tag_lc[from..].find(&key) {
        let at = from + i;
        // must start an attribute (preceded by whitespace)
        if at > 0 && !tag_lc.as_bytes()[at - 1].is_ascii_whitespace() {
            from = at + key.len();
            continue;
        }
        let rest = &tag[at + key.len()..];
        let (quote, body) = match rest.chars().next()? {
            q @ ('"' | '\'') => (Some(q), &rest[1..]),
            _ => (None, rest),
        };
        let val = match quote {
            Some(q) => body.split(q).next()?,
            None => body.split(|c: char| c.is_whitespace() || c == '>').next()?,
        };
        return Some(val.replace("&amp;", "&"));
    }
    None
}

fn inner_text(html: &str, lower: &str, from: usize) -> String {
    let end = lower[from..].find("</a").map(|i| from + i).unwrap_or(html.len().min(from + 200));
    let mut s = String::new();
    let mut in_tag = false;
    for c in html[from..end].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => s.push(c),
            _ => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn file_name(url: &str) -> String {
    Url::parse(url).ok().and_then(|u| u.path_segments().and_then(|s| s.filter(|p| !p.is_empty()).last().map(str::to_owned))).unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_lists() {
        let l = parse_url_list("# my files\nhttps://a.example/1.zip\n\n  http://b.example/2.zip  \nftp://nope/3\nnot a url\nhttps://a.example/1.zip\n");
        assert_eq!(l, vec!["https://a.example/1.zip", "http://b.example/2.zip"]);
    }

    #[test]
    fn page_links_are_resolved_cleaned_and_deduped() {
        let html = r##"<html><body>
          <a href="/files/a.zip">Archive A</a>
          <a href='b.pdf#page=2'><b>Doc</b>  B</a>
          <a href="https://other.example/c.iso"></a>
          <a href="#top">top</a><a href="mailto:x@y.z">mail</a><a href="javascript:void(0)">js</a>
          <a href="/files/a.zip">again</a>
          <a class="x" href="dl?id=1&amp;x=2">Download</a>
          <video src="/v/clip.mp4"></video>
        </body></html>"##;
        let l = extract_links(html, "https://site.example/dir/page.html");
        let urls: Vec<&str> = l.iter().map(|x| x.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://site.example/files/a.zip",
                "https://site.example/dir/b.pdf",
                "https://other.example/c.iso",
                "https://site.example/dir/dl?id=1&x=2",
                "https://site.example/v/clip.mp4"
            ]
        );
        assert_eq!(l[0].text, "Archive A");
        assert_eq!(l[1].text, "Doc B");
        assert_eq!(l[2].text, "c.iso", "no link text falls back to the file name");
    }
}
