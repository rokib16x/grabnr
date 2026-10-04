//! Credentials for a download, turned into an `Authorization` header.

use base64::Engine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Basic { user: String, pass: String },
    Bearer(String),
}

impl Auth {
    pub fn header_value(&self) -> String {
        match self {
            Auth::Basic { user, pass } => format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))),
            Auth::Bearer(t) => format!("Bearer {t}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_headers() {
        assert_eq!(Auth::Basic { user: "u".into(), pass: "p".into() }.header_value(), "Basic dTpw");
        assert_eq!(Auth::Bearer("abc".into()).header_value(), "Bearer abc");
    }
}
