//! `grabnr://add?url=<link>` links, so a web page or another app can hand downloads to grabnr.

const MAX_LINKS: usize = 20;

/// The http(s) links in a `grabnr://add?url=…&url=…` address. Anything else gives an empty list.
pub fn links_from(address: &str) -> Vec<String> {
    let Ok(u) = url::Url::parse(address) else { return Vec::new() };
    if u.scheme() != "grabnr" || u.host_str() != Some("add") {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for (k, v) in u.query_pairs() {
        if k == "url"
            && (v.starts_with("http://") || v.starts_with("https://"))
            && url::Url::parse(&v).is_ok()
            && !out.iter().any(|x| x == v.as_ref())
        {
            out.push(v.into_owned());
        }
        if out.len() == MAX_LINKS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_encoded_links() {
        let l = links_from("grabnr://add?url=https%3A%2F%2Fa.example%2Ffile.zip%3Fx%3D1&url=http%3A%2F%2Fb.example%2F2.iso");
        assert_eq!(l, ["https://a.example/file.zip?x=1", "http://b.example/2.iso"]);
    }

    #[test]
    fn ignores_other_schemes_actions_and_targets() {
        assert!(links_from("https://add?url=https://a/b").is_empty());
        assert!(links_from("grabnr://open?url=https://a/b").is_empty());
        assert!(links_from("grabnr://add?url=file%3A%2F%2F%2Fetc%2Fpasswd").is_empty());
        assert!(links_from("grabnr://add?url=javascript%3Aalert(1)").is_empty());
        assert!(links_from("not a url").is_empty());
    }

    #[test]
    fn dedupes_and_caps() {
        assert_eq!(links_from("grabnr://add?url=https://a/1&url=https://a/1").len(), 1);
        let many: String = (0..30).map(|i| format!("url=https://a/{i}&")).collect();
        assert_eq!(links_from(&format!("grabnr://add?{many}")).len(), MAX_LINKS);
    }
}
