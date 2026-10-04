//! Moving settings and the queue between machines: a JSON settings file and a plain list of links.

use crate::manager::{Item, Settings, SettingsPatch, Status};
use serde_json::{json, Value};

const FORMAT: u64 = 1;

/// A proxy address without the username and password in it.
fn strip_userinfo(proxy: &str) -> String {
    match url::Url::parse(proxy) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string().trim_end_matches('/').to_string()
        }
        Err(_) => String::new(),
    }
}

/// Settings worth carrying to another machine. The API token and any proxy credentials stay behind.
pub fn export_settings(s: &Settings) -> Value {
    json!({
        "grabnr_settings": FORMAT,
        "settings": {
            "dest_dir": s.dest_dir,
            "enabled_links": s.enabled_links,
            "conns_per_route": s.conns_per_route,
            "max_active": s.max_active,
            "speed_limit_kbps": s.speed_limit_kbps,
            "auto_retry": s.auto_retry,
            "link_rules": s.link_rules,
            "skip_cellular": s.skip_cellular,
            "proxy": strip_userinfo(&s.proxy),
            "sound": s.sound,
            "quarantine": s.quarantine,
            "schedule": s.schedule,
            "after_command": s.after_command,
            "webhook_url": s.webhook_url,
        }
    })
}

/// Read a settings file written by [`export_settings`]. Fields it does not know are ignored; the token and
/// the first-run flag are never imported.
pub fn import_settings(text: &str) -> Result<SettingsPatch, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "That is not a grabnr settings file.".to_string())?;
    if v.get("grabnr_settings").and_then(Value::as_u64) != Some(FORMAT) {
        return Err("That is not a grabnr settings file, or it comes from a newer version.".into());
    }
    let mut s = v.get("settings").cloned().ok_or("The settings file is empty.")?;
    if let Some(o) = s.as_object_mut() {
        o.remove("token");
        o.remove("onboarded");
    }
    serde_json::from_value(s).map_err(|e| format!("The settings file could not be read: {e}"))
}

/// Links still to be downloaded, one per line, ready to paste into Add Download on another machine.
pub fn queue_links(items: &[Item]) -> String {
    let mut out = String::new();
    for i in items.iter().rev().filter(|i| i.status != Status::Done) {
        out.push_str(&i.url);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Stats;
    use std::collections::HashMap;

    fn settings() -> Settings {
        Settings {
            dest_dir: "/Users/a/Downloads".into(),
            enabled_links: Some(vec!["en0".into()]),
            conns_per_route: 6,
            max_active: 2,
            token: "SECRET-TOKEN".into(),
            speed_limit_kbps: 1024,
            auto_retry: 4,
            link_rules: HashMap::new(),
            skip_cellular: true,
            proxy: "socks5://user:pass@127.0.0.1:1080".into(),
            onboarded: true,
            sound: false,
            quarantine: true,
            keychain: false,
            schedule: vec![],
            after_command: "echo done".into(),
            webhook_url: "https://hooks.example/x".into(),
        }
    }

    #[test]
    fn export_leaves_secrets_behind() {
        let text = export_settings(&settings()).to_string();
        assert!(!text.contains("SECRET-TOKEN") && !text.contains("user:pass") && !text.contains("pass@"), "{text}");
        assert!(text.contains("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn round_trips_and_never_imports_the_token() {
        let mut v = export_settings(&settings());
        v["settings"]["token"] = json!("attacker-token");
        v["settings"]["onboarded"] = json!(false);
        let p = import_settings(&v.to_string()).unwrap();
        assert_eq!(p.conns_per_route, Some(6));
        assert_eq!(p.enabled_links, Some(Some(vec!["en0".to_string()])));
        assert_eq!(p.sound, Some(false));
        assert_eq!(p.onboarded, None);
        assert_eq!(p.proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn null_enabled_links_means_every_link() {
        let mut s = settings();
        s.enabled_links = None;
        let p = import_settings(&export_settings(&s).to_string()).unwrap();
        assert_eq!(p.enabled_links, Some(None), "null must clear the list, not be ignored");
    }

    #[test]
    fn rejects_other_files() {
        assert!(import_settings("not json").is_err());
        assert!(import_settings(r#"{"hello":1}"#).is_err());
        assert!(import_settings(r#"{"grabnr_settings":2,"settings":{}}"#).is_err());
    }

    #[test]
    fn queue_list_skips_finished_and_keeps_oldest_first() {
        let mk = |id: &str, status| Item {
            id: id.into(),
            url: format!("https://x/{id}"),
            filename: None,
            dir: ".".into(),
            headers: vec![],
            status,
            total: None,
            downloaded: 0,
            path: None,
            error: None,
            added: 0,
            checksum: None,
            priority: 0,
            retries: 0,
            proxy: None,
            mirrors: vec![],
            stats: Stats::default(),
        };
        let items = vec![mk("new", Status::Queued), mk("done", Status::Done), mk("old", Status::Paused)];
        assert_eq!(queue_links(&items), "https://x/old\nhttps://x/new\n");
    }
}
