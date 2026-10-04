//! Keeping sign-in details out of the data files: with the Keychain option on, credentials live in the macOS
//! Keychain and the files hold only placeholders.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::manager::Item;

const PREFIX: &str = "keychain:";
const SENSITIVE_HEADERS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    fn available(&self) -> bool;
}

/// The macOS Keychain (a login-keychain item per secret).
#[cfg(target_os = "macos")]
pub struct Keychain;

#[cfg(target_os = "macos")]
impl Keychain {
    fn entry(key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("com.grabnr.app", key).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        Self::entry(key)?.set_password(value).map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn available(&self) -> bool {
        true
    }
}

/// Where there is no Keychain support.
#[cfg(not(target_os = "macos"))]
pub struct Unavailable;

#[cfg(not(target_os = "macos"))]
impl SecretStore for Unavailable {
    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Err("The Keychain is not available on this system".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("The Keychain is not available on this system".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn available(&self) -> bool {
        false
    }
}

/// Remembers what was last written so repeated saves do not touch the Keychain again.
pub struct Cached {
    inner: Box<dyn SecretStore>,
    last: Mutex<HashMap<String, String>>,
}

impl Cached {
    pub fn new(inner: Box<dyn SecretStore>) -> Self {
        Cached { inner, last: Mutex::new(HashMap::new()) }
    }
}

impl SecretStore for Cached {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.inner.get(key)
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self.last.lock().unwrap().get(key).is_some_and(|v| v == value) {
            return Ok(());
        }
        self.inner.set(key, value)?;
        self.last.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.last.lock().unwrap().remove(key);
        self.inner.delete(key)
    }
    fn available(&self) -> bool {
        self.inner.available()
    }
}

pub fn is_placeholder(v: &str) -> bool {
    v.starts_with(PREFIX)
}

fn has_credentials(proxy: &str) -> bool {
    url::Url::parse(proxy).map(|u| !u.username().is_empty() || u.password().is_some()).unwrap_or(false)
}

fn header_key(id: &str, name: &str) -> String {
    format!("{id}/h/{}", name.to_ascii_lowercase())
}

/// A copy of the items and proxy with every secret moved into the store and replaced by a placeholder.
pub fn scrub(items: &[Item], proxy: &str, store: &dyn SecretStore) -> Result<(Vec<Item>, String), String> {
    let mut out = items.to_vec();
    for it in &mut out {
        for (name, value) in &mut it.headers {
            if SENSITIVE_HEADERS.contains(&name.to_ascii_lowercase().as_str()) && !is_placeholder(value) {
                let key = header_key(&it.id, name);
                store.set(&key, value)?;
                *value = format!("{PREFIX}{key}");
            }
        }
        if let Some(p) = it.proxy.as_mut().filter(|p| has_credentials(p) && !is_placeholder(p)) {
            let key = format!("{}/proxy", it.id);
            store.set(&key, p)?;
            *p = format!("{PREFIX}{key}");
        }
    }
    let proxy = if has_credentials(proxy) {
        store.set("settings/proxy", proxy)?;
        format!("{PREFIX}settings/proxy")
    } else {
        proxy.to_string()
    };
    Ok((out, proxy))
}

fn resolve(value: &str, store: &dyn SecretStore) -> Option<String> {
    store.get(value.strip_prefix(PREFIX)?).ok().flatten()
}

/// Put real values back after loading. A secret that cannot be read is dropped (the server will then ask for
/// sign-in again). Returns how many could not be restored.
pub fn restore(items: &mut [Item], proxy: &mut String, store: &dyn SecretStore) -> usize {
    let mut lost = 0;
    for it in items.iter_mut() {
        it.headers.retain_mut(|(_, v)| {
            if !is_placeholder(v) {
                return true;
            }
            match resolve(v, store) {
                Some(real) => {
                    *v = real;
                    true
                }
                None => {
                    lost += 1;
                    false
                }
            }
        });
        if it.proxy.as_deref().is_some_and(is_placeholder) {
            it.proxy = it.proxy.as_deref().and_then(|p| resolve(p, store));
            if it.proxy.is_none() {
                lost += 1;
            }
        }
    }
    if is_placeholder(proxy) {
        *proxy = resolve(proxy, store).unwrap_or_default();
        if proxy.is_empty() {
            lost += 1;
        }
    }
    lost
}

/// Remove the Keychain entries that belong to one download.
pub fn forget(item: &Item, store: &dyn SecretStore) {
    for (name, _) in &item.headers {
        let _ = store.delete(&header_key(&item.id, name));
    }
    let _ = store.delete(&format!("{}/proxy", item.id));
}

/// Remove every Keychain entry grabnr wrote for these items (used when the option is turned off).
pub fn forget_all(items: &[Item], store: &dyn SecretStore) {
    items.iter().for_each(|i| forget(i, store));
    let _ = store.delete("settings/proxy");
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::history::Stats;
    use crate::manager::Status;

    #[derive(Default)]
    pub struct Memory(pub Mutex<HashMap<String, String>>);
    impl SecretStore for Memory {
        fn get(&self, k: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn set(&self, k: &str, v: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn delete(&self, k: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
        fn available(&self) -> bool {
            true
        }
    }

    fn item() -> Item {
        Item {
            id: "abc".into(),
            url: "https://x/y".into(),
            filename: None,
            dir: ".".into(),
            headers: vec![
                ("Authorization".into(), "Bearer s3cret".into()),
                ("Referer".into(), "https://x/".into()),
                ("Cookie".into(), "sid=1".into()),
            ],
            status: Status::Paused,
            total: None,
            downloaded: 0,
            path: None,
            error: None,
            added: 0,
            checksum: None,
            priority: 0,
            retries: 0,
            proxy: Some("socks5://u:p@h:1080".into()),
            mirrors: vec![],
            stats: Stats::default(),
        }
    }

    #[test]
    fn scrubbed_copies_hold_no_secrets_and_round_trip() {
        let store = Memory::default();
        let (scrubbed, proxy) = scrub(&[item()], "http://user:pw@proxy:8080", &store).unwrap();
        let text = format!("{:?}{proxy}", (&scrubbed[0].headers, &scrubbed[0].proxy));
        assert!(!text.contains("s3cret") && !text.contains("sid=1") && !text.contains("u:p@") && !text.contains("user:pw"), "{text}");
        assert_eq!(scrubbed[0].headers[1].1, "https://x/", "ordinary headers stay readable");

        let mut back = scrubbed.clone();
        let mut proxy2 = proxy.clone();
        assert_eq!(restore(&mut back, &mut proxy2, &store), 0);
        assert_eq!(back[0].headers, item().headers);
        assert_eq!(back[0].proxy.as_deref(), Some("socks5://u:p@h:1080"));
        assert_eq!(proxy2, "http://user:pw@proxy:8080");
    }

    #[test]
    fn a_proxy_without_credentials_stays_in_the_file() {
        let store = Memory::default();
        let (_, proxy) = scrub(&[], "socks5://127.0.0.1:1080", &store).unwrap();
        assert_eq!(proxy, "socks5://127.0.0.1:1080");
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn unreadable_secrets_are_dropped_not_sent_as_placeholders() {
        let store = Memory::default();
        let (mut scrubbed, mut proxy) = scrub(&[item()], "", &store).unwrap();
        store.0.lock().unwrap().clear(); // the Keychain entries were deleted behind our back
        let lost = restore(&mut scrubbed, &mut proxy, &store);
        assert_eq!(lost, 3, "authorization, cookie and the proxy");
        assert!(scrubbed[0].headers.iter().all(|(_, v)| !is_placeholder(v)));
        assert_eq!(scrubbed[0].headers.len(), 1);
        assert_eq!(scrubbed[0].proxy, None);
    }

    #[test]
    fn forgetting_removes_entries() {
        let store = Memory::default();
        scrub(&[item()], "", &store).unwrap();
        assert_eq!(store.0.lock().unwrap().len(), 3);
        forget(&item(), &store);
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn the_cache_skips_unchanged_writes() {
        use std::sync::Arc;
        struct Counting(Arc<Mutex<usize>>);
        impl SecretStore for Counting {
            fn get(&self, _: &str) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                *self.0.lock().unwrap() += 1;
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn available(&self) -> bool {
                true
            }
        }
        let writes = Arc::new(Mutex::new(0));
        let c = Cached::new(Box::new(Counting(writes.clone())));
        c.set("k", "v").unwrap();
        c.set("k", "v").unwrap();
        c.set("k", "w").unwrap();
        assert_eq!(*writes.lock().unwrap(), 2, "the repeated write of the same value must be skipped");
    }
}
