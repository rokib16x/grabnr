//! Owns the download list: persistence, queueing, pause/resume, and forwarding
//! engine events to the UI. Shared by the Tauri commands and the browser-extension API.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use grabnr_core::{interfaces::LinkKind, limiter::Limiter, list_links, Event, Options, Route, Store};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

use crate::history::{self, HistoryRec, Stats};
use crate::queue::{self, Entry};
use crate::schedule::{self, Effect, Rule};

pub const API_PORT: u16 = 17653;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Done,
    Error,
}

/// Persisted record. `headers` may hold cookies, so it never reaches the UI (see `ItemView`).
#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub url: String,
    pub filename: Option<String>,
    pub dir: String,
    pub headers: Vec<(String, String)>,
    pub status: Status,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    pub added: u64,
    /// Expected hash (`sha256:<hex>` etc.) the finished file is verified against.
    #[serde(default)]
    pub checksum: Option<String>,
    /// Higher starts first; ties go to the older download.
    #[serde(default)]
    pub priority: i32,
    /// Automatic retries used so far (reset on success or a manual resume).
    #[serde(default)]
    pub retries: u32,
    /// Proxy just for this download; may hold credentials, so it never reaches the UI.
    #[serde(default)]
    pub proxy: Option<String>,
    /// Other URLs serving the same file (from the add dialog or a Metalink file).
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// For HLS playlists: `best`, `worst` or a height such as `720`.
    #[serde(default)]
    pub quality: Option<String>,
    /// Bytes per link, running time and peak speed, for the history and statistics.
    #[serde(default)]
    pub stats: Stats,
}

#[derive(Serialize, Clone)]
pub struct ItemView {
    pub id: String,
    pub url: String,
    pub filename: Option<String>,
    pub dir: String,
    pub status: Status,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    pub added: u64,
    pub checksum: Option<String>,
    pub priority: i32,
    pub retries: u32,
}

impl From<&Item> for ItemView {
    fn from(i: &Item) -> Self {
        ItemView {
            id: i.id.clone(),
            url: i.url.clone(),
            filename: i.filename.clone(),
            dir: i.dir.clone(),
            status: i.status,
            total: i.total,
            downloaded: i.downloaded,
            path: i.path.clone(),
            error: i.error.clone(),
            added: i.added,
            checksum: i.checksum.clone(),
            priority: i.priority,
            retries: i.retries,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub dest_dir: String,
    /// `None` means every active non-VPN link.
    pub enabled_links: Option<Vec<String>>,
    pub conns_per_route: usize,
    pub max_active: usize,
    pub token: String,
    /// Total download speed cap in KB/s across all links; 0 means unlimited.
    #[serde(default)]
    pub speed_limit_kbps: u64,
    /// Times a failed download is retried automatically (with growing delays); 0 turns it off.
    #[serde(default = "default_auto_retry")]
    pub auto_retry: u32,
    /// Per-link share of connections and speed cap, keyed by interface name.
    #[serde(default)]
    pub link_rules: HashMap<String, LinkRule>,
    /// Leave cellular links out unless they are ticked explicitly.
    #[serde(default)]
    pub skip_cellular: bool,
    /// Proxy for every download that has none of its own; empty means direct.
    #[serde(default)]
    pub proxy: String,
    /// The first-run setup has been shown.
    #[serde(default)]
    pub onboarded: bool,
    /// Play the system sound with the "download complete" notification.
    #[serde(default = "yes")]
    pub sound: bool,
    /// Time-of-day rules (pause, cap the speed, or lift the cap).
    #[serde(default)]
    pub schedule: Vec<Rule>,
    /// Shell command for the "run a command when downloads finish" action.
    #[serde(default)]
    pub after_command: String,
    /// POSTed a small JSON event whenever a download finishes or fails; empty for none.
    #[serde(default)]
    pub webhook_url: String,
    /// Mark finished files as downloaded from the internet (macOS), so Gatekeeper checks apps and installers.
    #[serde(default = "yes")]
    pub quarantine: bool,
    /// Keep sign-in headers and proxy passwords in the macOS Keychain instead of the data files.
    #[serde(default)]
    pub keychain: bool,
    /// Private key file for `sftp://` links (the password in the link is then its passphrase). Empty to sign in with a password.
    #[serde(default)]
    pub ssh_key: String,
    /// Turn downloaded HLS videos into MP4 files when ffmpeg is installed.
    #[serde(default = "yes")]
    pub hls_to_mp4: bool,
}

/// What to do once the queue has finished. Never saved: a forgotten "sleep" must not surprise the next session.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum AfterAll {
    #[default]
    None,
    Sleep,
    Quit,
    Command,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LinkRule {
    /// Percent of `conns_per_route` this link may open (10-100).
    #[serde(default = "full_share")]
    pub share_pct: u8,
    /// Speed cap for this link in KB/s; 0 means unlimited.
    #[serde(default)]
    pub limit_kbps: u64,
}

fn full_share() -> u8 {
    100
}

fn default_auto_retry() -> u32 {
    3
}

/// `null` and "missing" are different for `Option<Option<T>>`: null clears the value, missing leaves it alone.
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

#[derive(Deserialize)]
pub struct SettingsPatch {
    pub dest_dir: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub enabled_links: Option<Option<Vec<String>>>,
    pub conns_per_route: Option<usize>,
    pub max_active: Option<usize>,
    pub speed_limit_kbps: Option<u64>,
    pub auto_retry: Option<u32>,
    pub link_rules: Option<HashMap<String, LinkRule>>,
    pub skip_cellular: Option<bool>,
    pub proxy: Option<String>,
    pub onboarded: Option<bool>,
    pub sound: Option<bool>,
    pub schedule: Option<Vec<Rule>>,
    pub after_command: Option<String>,
    pub webhook_url: Option<String>,
    pub quarantine: Option<bool>,
    pub keychain: Option<bool>,
    pub hls_to_mp4: Option<bool>,
    pub ssh_key: Option<String>,
}

pub struct AddRequest {
    pub url: String,
    pub filename: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Save folder for this download; falls back to the default in settings.
    pub dir: Option<String>,
    pub checksum: Option<String>,
    pub proxy: Option<String>,
    pub mirrors: Vec<String>,
    pub quality: Option<String>,
}

struct Inner {
    items: Vec<Item>,
    settings: Settings,
    cancels: HashMap<String, CancellationToken>,
    /// Ids being removed, so a cancel is not reported as "paused".
    removing: Vec<String>,
    speeds: HashMap<String, f64>,
    /// Failed downloads waiting out their retry delay.
    retry_at: HashMap<String, Instant>,
    /// The current run of each running download (engine byte counters restart every run).
    runs: HashMap<String, Run>,
    /// The schedule is holding downloads back.
    hold: bool,
    /// Downloads stopped by the schedule, to be queued again rather than shown as paused.
    held: HashSet<String>,
    after_all: AfterAll,
}

struct Run {
    started: Instant,
    seen: HashMap<String, u64>,
}

pub struct Manager {
    app: AppHandle,
    data_dir: PathBuf,
    store: Arc<Store>,
    inner: Mutex<Inner>,
    pairing_until: Mutex<Option<Instant>>,
    history: Mutex<Vec<HistoryRec>>,
    /// One speed limit shared by every download; the schedule and settings change it while they run.
    webhook_error: Mutex<Option<String>>,
    secrets: Arc<crate::secrets::Cached>,
    keychain_error: Mutex<Option<String>>,
    limiter: Arc<Limiter>,
    effect: Mutex<Effect>,
    pub api_ok: Mutex<bool>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("os randomness");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Write a file only the owner can read (it can contain cookies and the API token).
fn write_private(path: &PathBuf, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl Manager {
    pub fn load(app: AppHandle) -> Result<Arc<Self>, String> {
        let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let store = Arc::new(Store::open(&data_dir.join("state.db")).map_err(|e| e.to_string())?);

        let settings = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_else(|| Settings {
                dest_dir: app.path().download_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into()),
                enabled_links: None,
                conns_per_route: 8,
                max_active: 3,
                token: random_hex(24),
                speed_limit_kbps: 0,
                auto_retry: default_auto_retry(),
                link_rules: HashMap::new(),
                skip_cellular: false,
                proxy: String::new(),
                onboarded: false,
                sound: true,
                schedule: Vec::new(),
                after_command: String::new(),
                webhook_url: String::new(),
                quarantine: true,
                keychain: false,
                hls_to_mp4: true,
                ssh_key: String::new(),
            });
        #[cfg(target_os = "macos")]
        let backend: Box<dyn crate::secrets::SecretStore> = Box::new(crate::secrets::Keychain);
        #[cfg(not(target_os = "macos"))]
        let backend: Box<dyn crate::secrets::SecretStore> = Box::new(crate::secrets::Unavailable);
        let secrets = Arc::new(crate::secrets::Cached::new(backend));
        let mut items: Vec<Item> =
            std::fs::read(data_dir.join("downloads.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut settings = settings;
        // Sign-in details kept in the Keychain come back here; ones that cannot be read are dropped.
        crate::secrets::restore(&mut items, &mut settings.proxy, &*secrets);
        // Anything that was running when the app quit is resumable, not running.
        for i in &mut items {
            if matches!(i.status, Status::Downloading | Status::Queued) {
                i.status = Status::Paused;
            }
        }
        let history: Vec<HistoryRec> =
            std::fs::read(data_dir.join("history.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let m = Arc::new(Manager {
            app,
            data_dir,
            store,
            inner: Mutex::new(Inner {
                items,
                settings,
                cancels: HashMap::new(),
                removing: Vec::new(),
                speeds: HashMap::new(),
                retry_at: HashMap::new(),
                runs: HashMap::new(),
                hold: false,
                held: HashSet::new(),
                after_all: AfterAll::None,
            }),
            pairing_until: Mutex::new(None),
            history: Mutex::new(history),
            webhook_error: Mutex::new(None),
            secrets,
            keychain_error: Mutex::new(None),
            limiter: Arc::new(Limiter::new(0)),
            effect: Mutex::new(Effect::default()),
            api_ok: Mutex::new(false),
        });
        m.persist();
        m.apply_schedule_now();
        Ok(m)
    }

    fn persist(&self) {
        let g = self.inner.lock().unwrap();
        let mut settings = g.settings.clone();
        let mut items = g.items.clone();
        if g.settings.keychain {
            match crate::secrets::scrub(&g.items, &g.settings.proxy, &*self.secrets) {
                Ok((i, p)) => {
                    items = i;
                    settings.proxy = p;
                    *self.keychain_error.lock().unwrap() = None;
                }
                // Better to keep the data than to lose it: fall back to the file and say so in the diagnostics.
                Err(e) => *self.keychain_error.lock().unwrap() = Some(e),
            }
        }
        if let Ok(b) = serde_json::to_vec_pretty(&items) {
            let _ = write_private(&self.data_dir.join("downloads.json"), &b);
        }
        if let Ok(b) = serde_json::to_vec_pretty(&settings) {
            let _ = write_private(&self.data_dir.join("settings.json"), &b);
        }
    }

    pub fn keychain_available(&self) -> bool {
        crate::secrets::SecretStore::available(&*self.secrets)
    }

    /// Check that the Keychain accepts a write and gives it back, before the option is switched on.
    pub fn keychain_probe(&self) -> Result<(), String> {
        use crate::secrets::SecretStore;
        let key = "probe/grabnr";
        self.secrets.set(key, "ok").map_err(|e| format!("The Keychain refused access: {e}"))?;
        let back = self.secrets.get(key).map_err(|e| format!("The Keychain could not be read: {e}"))?;
        let _ = self.secrets.delete(key);
        if back.as_deref() == Some("ok") {
            Ok(())
        } else {
            Err("The Keychain did not return what was stored.".into())
        }
    }

    pub fn list(&self) -> Vec<ItemView> {
        self.inner.lock().unwrap().items.iter().map(ItemView::from).collect()
    }

    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }

    pub fn set_settings(self: &Arc<Self>, p: SettingsPatch) {
        let mut turned_off_keychain = false;
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(v) = p.dest_dir {
                g.settings.dest_dir = v;
            }
            if let Some(v) = p.enabled_links {
                g.settings.enabled_links = v;
            }
            if let Some(v) = p.conns_per_route {
                g.settings.conns_per_route = v.clamp(1, 32);
            }
            if let Some(v) = p.max_active {
                g.settings.max_active = v.clamp(1, 10);
            }
            if let Some(v) = p.speed_limit_kbps {
                g.settings.speed_limit_kbps = v;
            }
            if let Some(v) = p.auto_retry {
                g.settings.auto_retry = v.min(10);
            }
            if let Some(mut v) = p.link_rules {
                for r in v.values_mut() {
                    r.share_pct = r.share_pct.clamp(10, 100);
                }
                g.settings.link_rules = v;
            }
            if let Some(v) = p.skip_cellular {
                g.settings.skip_cellular = v;
            }
            if let Some(v) = p.proxy {
                g.settings.proxy = v.trim().to_string();
            }
            if let Some(v) = p.onboarded {
                g.settings.onboarded = v;
            }
            if let Some(v) = p.sound {
                g.settings.sound = v;
            }
            if let Some(v) = p.schedule {
                g.settings.schedule = v;
            }
            if let Some(v) = p.after_command {
                g.settings.after_command = v;
            }
            if let Some(v) = p.webhook_url {
                g.settings.webhook_url = v.trim().to_string();
            }
            if let Some(v) = p.quarantine {
                g.settings.quarantine = v;
            }
            if let Some(v) = p.ssh_key {
                g.settings.ssh_key = v.trim().to_string();
            }
            if let Some(v) = p.hls_to_mp4 {
                g.settings.hls_to_mp4 = v;
            }
            if let Some(v) = p.keychain {
                turned_off_keychain = g.settings.keychain && !v;
                g.settings.keychain = v;
            }
        }
        self.persist();
        if turned_off_keychain {
            // The files now hold the real values again; remove the copies from the Keychain.
            let items = self.inner.lock().unwrap().items.clone();
            crate::secrets::forget_all(&items, &*self.secrets);
        }
        self.apply_schedule_now();
    }

    pub fn token_ok(&self, t: &str) -> bool {
        let real = self.inner.lock().unwrap().settings.token.clone();
        real.len() == t.len() && real.bytes().zip(t.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
    }

    pub fn allow_pairing(&self, secs: u64) {
        *self.pairing_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(secs));
    }

    pub fn pairing_token(&self) -> Option<String> {
        let open = self.pairing_until.lock().unwrap().is_some_and(|t| Instant::now() < t);
        open.then(|| self.settings().token)
    }

    pub fn paired(&self) -> bool {
        // "Paired" just means a token exists; the extension proves it with /add.
        true
    }

    /// A failed download whose link has probably expired and that this new link replaces: same site and path, only the
    /// query (a signature or token) differs, and the old attempt ended with an authorization error.
    pub fn relink_target(items: &[Item], new_url: &str, new_filename: Option<&str>) -> Option<String> {
        let (host, path) = split_url(new_url)?;
        items.iter().find_map(|i| {
            let (h, p) = split_url(&i.url)?;
            let expired =
                i.status == Status::Error && i.error.as_deref().is_some_and(|e| ["401", "403", "410"].iter().any(|c| e.contains(c)));
            let same_file = match (new_filename, i.filename.as_deref()) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            };
            (expired && h == host && p == path && i.url != new_url && same_file).then(|| i.id.clone())
        })
    }

    /// Add a download. A link already in the list is not added twice: the existing one is returned (and restarted if it had failed or paused).
    pub fn add(self: &Arc<Self>, req: AddRequest) -> (String, bool) {
        let relink = Self::relink_target(&self.inner.lock().unwrap().items, &req.url, req.filename.as_deref());
        if let Some(id) = relink {
            // The browser handed over a fresh link for a download that failed with an expired one: carry on with it.
            if self.update_link(&id, &req.url).is_ok() {
                return (id, true);
            }
        }
        let dup = {
            let g = self.inner.lock().unwrap();
            g.items
                .iter()
                .find(|i| i.url == req.url && i.dir == req.dir.clone().unwrap_or_else(|| g.settings.dest_dir.clone()))
                .map(|i| (i.id.clone(), i.status))
        };
        if let Some((id, status)) = dup {
            if matches!(status, Status::Paused | Status::Error) {
                self.resume(&id);
            }
            return (id, true);
        }
        let id = random_hex(8);
        {
            let mut g = self.inner.lock().unwrap();
            let dir = req.dir.filter(|d| !d.trim().is_empty()).unwrap_or_else(|| g.settings.dest_dir.clone());
            g.items.insert(
                0,
                Item {
                    id: id.clone(),
                    url: req.url,
                    filename: req.filename.filter(|f| !f.is_empty()),
                    dir,
                    headers: req.headers,
                    status: Status::Queued,
                    total: None,
                    downloaded: 0,
                    path: None,
                    error: None,
                    added: now(),
                    checksum: req.checksum.filter(|c| !c.trim().is_empty()),
                    priority: 0,
                    retries: 0,
                    proxy: req.proxy.filter(|p| !p.trim().is_empty()),
                    mirrors: req.mirrors,
                    quality: req.quality.filter(|q| !q.trim().is_empty()),
                    stats: Stats::default(),
                },
            );
        }
        self.persist();
        self.emit_item(&id);
        self.pump();
        (id, false)
    }

    /// Point a stopped download at a new link (for example a fresh signed URL) and carry on where it left off.
    /// The server must still report the same file; if not, the engine starts over by itself.
    pub fn update_link(self: &Arc<Self>, id: &str, url: &str) -> Result<(), String> {
        let url = url.trim();
        if !grabnr_core::links::downloadable(url) {
            return Err("Enter a full http://, https://, ftp:// or sftp:// link".into());
        }
        {
            let mut g = self.inner.lock().unwrap();
            let i = g.items.iter_mut().find(|i| i.id == id).ok_or("That download no longer exists")?;
            if matches!(i.status, Status::Downloading | Status::Done) {
                return Err("Only a paused, waiting or failed download can get a new link".into());
            }
            i.url = url.to_string();
            i.mirrors.clear();
        }
        self.resume(id);
        Ok(())
    }

    pub fn pause(&self, id: &str) {
        let c = self.inner.lock().unwrap().cancels.get(id).cloned();
        match c {
            Some(c) => c.cancel(),
            None => {
                // Queued items just become paused.
                self.set_status(id, Status::Paused, None);
            }
        }
    }

    pub fn resume(self: &Arc<Self>, id: &str) {
        {
            let mut g = self.inner.lock().unwrap();
            g.retry_at.remove(id);
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                i.retries = 0;
            }
        }
        self.set_status(id, Status::Queued, None);
        self.pump();
    }

    pub fn remove(&self, id: &str, delete_files: bool) {
        let (cancel, item) = {
            let mut g = self.inner.lock().unwrap();
            g.removing.push(id.to_string());
            (g.cancels.get(id).cloned(), g.items.iter().find(|i| i.id == id).cloned())
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        if let Some(item) = &item {
            crate::secrets::forget(item, &*self.secrets);
        }
        if let Some(item) = item {
            if delete_files {
                if let Some(p) = &item.path {
                    let _ = std::fs::remove_file(p);
                } else if let Some(f) = &item.filename {
                    let _ = std::fs::remove_file(PathBuf::from(&item.dir).join(format!("{f}.grabnr")));
                }
            }
        }
        self.inner.lock().unwrap().items.retain(|i| i.id != id);
        self.persist();
        let _ = self.app.emit("item-removed", id);
    }

    /// (running downloads, combined bytes/s) for the menu bar.
    pub fn summary(&self) -> (usize, f64) {
        let g = self.inner.lock().unwrap();
        (g.cancels.len(), g.speeds.values().sum())
    }

    /// Make a queued download start next (or last) by changing its priority.
    pub fn move_item(self: &Arc<Self>, id: &str, to_front: bool) {
        {
            let mut g = self.inner.lock().unwrap();
            let (hi, lo) = (g.items.iter().map(|i| i.priority).max().unwrap_or(0), g.items.iter().map(|i| i.priority).min().unwrap_or(0));
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                i.priority = if to_front { hi + 1 } else { lo - 1 };
            }
        }
        self.persist();
        self.emit_item(id);
        self.pump();
    }

    pub fn pause_all(&self) {
        let ids: Vec<String> = self
            .inner
            .lock()
            .unwrap()
            .items
            .iter()
            .filter(|i| matches!(i.status, Status::Downloading | Status::Queued))
            .map(|i| i.id.clone())
            .collect();
        for id in ids {
            self.pause(&id);
        }
    }

    pub fn resume_all(self: &Arc<Self>) {
        let ids: Vec<String> =
            self.inner.lock().unwrap().items.iter().filter(|i| i.status == Status::Paused).map(|i| i.id.clone()).collect();
        for id in ids {
            self.set_status(&id, Status::Queued, None);
        }
        self.pump();
    }

    pub fn find_path(&self, id: &str) -> Option<PathBuf> {
        let g = self.inner.lock().unwrap();
        let i = g.items.iter().find(|i| i.id == id)?;
        Some(i.path.as_ref().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&i.dir)))
    }

    fn set_status(&self, id: &str, status: Status, error: Option<String>) {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                i.status = status;
                i.error = error;
            }
        }
        self.persist();
        self.emit_item(id);
    }

    fn emit_item(&self, id: &str) {
        let v = self.inner.lock().unwrap().items.iter().find(|i| i.id == id).map(ItemView::from);
        if let Some(v) = v {
            let _ = self.app.emit("item-updated", v);
        }
    }

    /// Start queued downloads until `max_active` are running.
    fn pump(self: &Arc<Self>) {
        loop {
            let next = {
                let g = self.inner.lock().unwrap();
                let active = g.cancels.len();
                if g.hold || active >= g.settings.max_active {
                    None
                } else {
                    pick_next(&g.items, &g.cancels, &g.retry_at, Instant::now())
                }
            };
            let Some(item) = next else { return };
            self.start(item);
        }
    }

    /// The links to use right now, per settings. May be empty (no network).
    fn link_routes(&self) -> Vec<Route> {
        let (enabled, rules, skip_cellular, conns) = {
            let g = self.inner.lock().unwrap();
            (g.settings.enabled_links.clone(), g.settings.link_rules.clone(), g.settings.skip_cellular, g.settings.conns_per_route)
        };
        let routes: Vec<Route> = list_links()
            .iter()
            .filter(|l| match &enabled {
                Some(names) => names.contains(&l.name),
                None => l.kind != LinkKind::Tunnel && !(skip_cellular && l.kind == LinkKind::Cellular),
            })
            .map(|l| {
                let mut r = Route::from_link(l);
                if let Some(rule) = rules.get(&l.name) {
                    r.max_conns = Some((conns * rule.share_pct.clamp(10, 100) as usize).div_ceil(100).max(1));
                    r.speed_limit = (rule.limit_kbps > 0).then(|| rule.limit_kbps * 1024);
                }
                r
            })
            .collect();
        routes
    }

    /// Like `link_routes`, but falls back to the OS default route so a download can still start with no usable link.
    fn routes(&self) -> Vec<Route> {
        let routes = self.link_routes();
        if routes.is_empty() {
            vec![Route::unbound("default")]
        } else {
            routes
        }
    }

    fn start(self: &Arc<Self>, item: Item) {
        let (conns, global_proxy, cancel) = {
            let mut g = self.inner.lock().unwrap();
            let c = CancellationToken::new();
            g.cancels.insert(item.id.clone(), c.clone());
            g.runs.insert(item.id.clone(), Run { started: Instant::now(), seen: HashMap::new() });
            (g.settings.conns_per_route, g.settings.proxy.clone(), c)
        };
        self.set_status(&item.id, Status::Downloading, None);

        let mut headers = item.headers.clone();
        let mut url = item.url.clone();
        if grabnr_core::is_remote(&url) {
            // FTP and SFTP sign in through the link itself; the stored link and the Authorization header never held the
            // password in the clear, so rebuild it here, just for the engine.
            if let Some(i) = headers.iter().position(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
                if let Some((u, p)) = basic_credentials(&headers[i].1) {
                    url = with_userinfo(&url, &u, &p);
                }
                headers.remove(i);
            }
        }
        let (ssh_key, known_hosts) = {
            let g = self.inner.lock().unwrap();
            (Some(PathBuf::from(&g.settings.ssh_key)).filter(|p| !p.as_os_str().is_empty()), self.data_dir.join("known_hosts"))
        };
        let mut opts = Options::new(url, &item.dir, self.routes());
        opts.ssh = grabnr_core::sftp::SshOptions { known_hosts: Some(known_hosts), key: ssh_key };
        opts.filename = item.filename.clone();
        opts.headers = headers;
        opts.conns_per_route = conns;
        opts.store = Some(self.store.clone());
        // Keyed by the download, not its link, so a replaced link keeps the partial file.
        opts.resume_key = Some(item.id.clone());
        // Links that come and go (cable plugged in, Wi-Fi back after sleep) join or leave this download.
        let watcher = self.clone();
        opts.link_watch = Some(Arc::new(move || watcher.link_routes()));
        opts.shared_limit = Some(self.limiter.clone());
        opts.mirrors = item.mirrors.clone();
        opts.hls_quality = item.quality.as_deref().map(grabnr_core::Quality::parse).unwrap_or_default();
        if self.inner.lock().unwrap().settings.hls_to_mp4 {
            opts.ffmpeg = find_ffmpeg();
        }
        opts.proxy = item.proxy.clone().or_else(|| Some(global_proxy).filter(|p| !p.is_empty()));
        if let Some(c) = &item.checksum {
            match grabnr_core::Checksum::parse(c) {
                Ok(sum) => opts.checksum = Some(sum),
                Err(e) => {
                    self.finish(&item.id, Err(e));
                    return;
                }
            }
        }

        let me = self.clone();
        let id = item.id.clone();
        tauri::async_runtime::spawn(async move {
            let (m2, id2) = (me.clone(), id.clone());
            let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| m2.on_event(&id2, e));
            let res = grabnr_core::download(opts, cancel, emit).await;
            me.finish(&id, res);
        });
    }

    fn on_event(&self, id: &str, e: Event) {
        let mut changed = false;
        let mut speed = None;
        {
            let mut g = self.inner.lock().unwrap();
            let Inner { items, runs, .. } = &mut *g;
            if let Some(i) = items.iter_mut().find(|i| i.id == id) {
                match &e {
                    Event::Started { filename, total, .. } => {
                        i.filename.get_or_insert_with(|| filename.clone());
                        i.total = *total;
                        changed = true;
                    }
                    Event::Progress(s) => {
                        i.downloaded = s.downloaded;
                        // Streams (HLS) only know their size by estimate, which improves as segments arrive.
                        if s.total.is_some() && s.total != i.total {
                            i.total = s.total;
                            changed = true;
                        }
                        speed = Some(s.bytes_per_sec);
                        if let Some(run) = runs.get_mut(id) {
                            i.stats.add_progress(&mut run.seen, &s.routes);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(v) = speed {
                g.speeds.insert(id.to_string(), v);
            }
        }
        if changed {
            self.emit_item(id);
        }
        let _ = self.app.emit("download-event", serde_json::json!({ "id": id, "event": e }));
    }

    /// Decide whether a failure should be retried; returns the wait before the next attempt.
    fn schedule_retry(&self, id: &str, e: &grabnr_core::Error, msg: &str) -> Option<Duration> {
        if !is_transient(e) {
            return None;
        }
        let delay = {
            let mut g = self.inner.lock().unwrap();
            let max = g.settings.auto_retry;
            let item = g.items.iter_mut().find(|i| i.id == id)?;
            if item.retries >= max {
                return None;
            }
            item.retries += 1;
            let d = retry_delay(item.retries);
            g.retry_at.insert(id.to_string(), Instant::now() + d);
            d
        };
        self.set_status(id, Status::Queued, Some(format!("{msg}. Retrying in {}s", delay.as_secs())));
        Some(delay)
    }

    /// Tell the configured webhook (if any) that a download finished or failed. Fire and forget.
    fn webhook(self: &Arc<Self>, event: &str, id: &str, error: Option<String>) {
        let (url, proxy, body) = {
            let g = self.inner.lock().unwrap();
            if g.settings.webhook_url.is_empty() {
                return;
            }
            let Some(i) = g.items.iter().find(|i| i.id == id) else { return };
            let body = serde_json::json!({
                "event": event,
                "id": i.id,
                "name": i.filename,
                "url": i.url,
                "bytes": i.total.unwrap_or(i.downloaded),
                "path": i.path,
                "error": error,
                "time": now(),
            });
            (g.settings.webhook_url.clone(), Some(g.settings.proxy.clone()).filter(|p| !p.is_empty()), body.to_string())
        };
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let res = grabnr_core::fetch::post_json(&url, body, proxy.as_deref(), Duration::from_secs(10)).await;
            *me.webhook_error.lock().unwrap() = res.err().map(|e| e.to_string());
        });
    }

    pub fn queue_links(&self) -> String {
        crate::portable::queue_links(&self.inner.lock().unwrap().items)
    }

    /// Finished file and the folder for its cached preview.
    pub fn thumbnail_source(&self, id: &str) -> Option<(PathBuf, PathBuf)> {
        let g = self.inner.lock().unwrap();
        let i = g.items.iter().find(|i| i.id == id && i.status == Status::Done)?;
        Some((PathBuf::from(i.path.as_ref()?), self.data_dir.join("thumbs")))
    }

    pub fn diagnostics(&self, links: &[grabnr_core::Link]) -> String {
        use crate::report::{build, host_only, Counts, Input};
        let g = self.inner.lock().unwrap();
        let count = |s: Status| g.items.iter().filter(|i| i.status == s).count();
        let errors: Vec<(String, String)> = g
            .items
            .iter()
            .filter(|i| i.status == Status::Error)
            .take(20)
            .map(|i| (host_only(&i.url), i.error.clone().unwrap_or_default()))
            .collect();
        let link_rows: Vec<(String, String, String)> =
            links.iter().map(|l| (l.name.clone(), l.label.clone(), format!("{:?}", l.kind))).collect();
        let webhook_error = self.webhook_error.lock().unwrap().clone();
        build(&Input {
            version: env!("CARGO_PKG_VERSION"),
            os: &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            links: &link_rows,
            settings: &g.settings,
            counts: Counts {
                queued: count(Status::Queued),
                downloading: count(Status::Downloading),
                paused: count(Status::Paused),
                done: count(Status::Done),
                error: count(Status::Error),
            },
            errors: &errors,
            webhook_error: webhook_error.as_deref(),
        })
    }

    /// Apply the schedule and general limit for the current local time.
    pub fn apply_schedule_now(self: &Arc<Self>) {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        self.apply_schedule(now.weekday().num_days_from_monday() as usize, (now.hour() * 60 + now.minute()) as u16);
    }

    fn apply_schedule(self: &Arc<Self>, weekday: usize, minute: u16) {
        let (effect, entering_hold, leaving_hold) = {
            let mut g = self.inner.lock().unwrap();
            let e = schedule::effect(&g.settings.schedule, g.settings.speed_limit_kbps, weekday, minute);
            let entering = e.hold && !g.hold;
            let leaving = !e.hold && g.hold;
            g.hold = e.hold;
            if entering {
                // Stop what is running; each one goes back to the queue instead of showing as paused.
                let running: Vec<String> = g.cancels.keys().cloned().collect();
                for id in running {
                    if let Some(c) = g.cancels.get(&id) {
                        c.cancel();
                    }
                    g.held.insert(id);
                }
            }
            (e, entering, leaving)
        };
        self.limiter.set_rate(effect.limit_kbps * 1024);
        let changed = {
            let mut cur = self.effect.lock().unwrap();
            let changed = *cur != effect;
            *cur = effect.clone();
            changed
        };
        if changed || entering_hold || leaving_hold {
            let _ = self.app.emit("schedule-changed", &effect);
        }
        if leaving_hold {
            self.pump();
        }
    }

    pub fn schedule_effect(&self) -> Effect {
        self.effect.lock().unwrap().clone()
    }

    pub fn after_all(&self) -> AfterAll {
        self.inner.lock().unwrap().after_all
    }

    pub fn set_after_all(&self, a: AfterAll) {
        self.inner.lock().unwrap().after_all = a;
        let _ = self.app.emit("after-all-changed", a);
    }

    /// When the last waiting or running download has finished, carry out the chosen action after a short delay
    /// (so it can still be cancelled), then forget it.
    fn maybe_run_after_all(self: &Arc<Self>) {
        let (action, command) = {
            let g = self.inner.lock().unwrap();
            let busy = g.items.iter().any(|i| matches!(i.status, Status::Queued | Status::Downloading));
            if busy || g.hold || g.after_all == AfterAll::None {
                return;
            }
            (g.after_all, g.settings.after_command.clone())
        };
        let _ = self
            .app
            .notification()
            .builder()
            .title("All downloads finished")
            .body("grabnr will carry out your \"when finished\" action in 10 seconds.")
            .show();
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            {
                let mut g = me.inner.lock().unwrap();
                let still = g.after_all == action && !g.items.iter().any(|i| matches!(i.status, Status::Queued | Status::Downloading));
                if !still {
                    return; // cancelled, or something new started
                }
                g.after_all = AfterAll::None;
            }
            let _ = me.app.emit("after-all-changed", AfterAll::None);
            match action {
                AfterAll::Quit => me.app.exit(0),
                AfterAll::Sleep => {
                    let _ = sleep_command().spawn();
                }
                AfterAll::Command if !command.trim().is_empty() => {
                    let _ = shell_command(&command).env("GRABNR_EVENT", "queue-finished").spawn();
                }
                _ => {}
            }
        });
    }

    /// Put `id` in front of `before` in the queue (or last when `None`), as dragged in the list.
    pub fn reorder(&self, id: &str, before: Option<&str>) {
        let changed: Vec<String> = {
            let mut g = self.inner.lock().unwrap();
            let entries: Vec<Entry> =
                g.items.iter().enumerate().map(|(age, i)| Entry { id: i.id.clone(), priority: i.priority, age }).collect();
            let ordered = queue::move_before(queue::order(&entries), id, before);
            let mut changed = Vec::new();
            for (pid, p) in queue::priorities(&ordered) {
                if let Some(it) = g.items.iter_mut().find(|i| i.id == pid) {
                    if it.priority != p {
                        it.priority = p;
                        changed.push(pid);
                    }
                }
            }
            changed
        };
        self.persist();
        for id in changed {
            self.emit_item(&id);
        }
    }

    fn add_history(&self, id: &str) {
        let rec = {
            let g = self.inner.lock().unwrap();
            g.items.iter().find(|i| i.id == id).map(|i| {
                history::record(&i.id, i.filename.as_deref().unwrap_or(&i.url), &i.url, i.total.unwrap_or(i.downloaded), now(), &i.stats)
            })
        };
        if let Some(rec) = rec {
            let mut h = self.history.lock().unwrap();
            h.retain(|r| r.id != rec.id);
            h.push(rec);
            let excess = h.len().saturating_sub(5000);
            h.drain(..excess);
            if let Ok(b) = serde_json::to_vec_pretty(&*h) {
                let _ = write_private(&self.data_dir.join("history.json"), &b);
            }
        }
    }

    pub fn history(&self) -> Vec<HistoryRec> {
        let mut h = self.history.lock().unwrap().clone();
        h.reverse();
        h
    }

    pub fn clear_history(&self) {
        self.history.lock().unwrap().clear();
        let _ = std::fs::remove_file(self.data_dir.join("history.json"));
    }

    pub fn history_csv(&self) -> String {
        history::to_csv(&self.history())
    }

    /// Combined progress (0-100) of everything downloading with a known size, for the Dock icon.
    pub fn overall_progress(&self) -> Option<u64> {
        let g = self.inner.lock().unwrap();
        let (mut done, mut total) = (0u64, 0u64);
        for i in g.items.iter().filter(|i| i.status == Status::Downloading) {
            if let Some(t) = i.total {
                total += t;
                done += i.downloaded.min(t);
            }
        }
        (total > 0).then(|| done * 100 / total)
    }

    fn finish(self: &Arc<Self>, id: &str, res: grabnr_core::Result<PathBuf>) {
        let removed = {
            let mut g = self.inner.lock().unwrap();
            g.cancels.remove(id);
            g.speeds.remove(id);
            if let Some(secs) = g.runs.remove(id).map(|r| r.started.elapsed().as_secs_f64()) {
                if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                    i.stats.active_secs += secs;
                }
            }
            let held = g.held.remove(id);
            (g.removing.iter().position(|r| r == id).map(|p| g.removing.remove(p)).is_some(), held)
        };
        let (removed, held) = removed;
        if !removed {
            match res {
                Ok(path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    {
                        let mut g = self.inner.lock().unwrap();
                        if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                            i.path = Some(path.display().to_string());
                            i.filename = Some(name.clone());
                            i.retries = 0;
                            if let Some(t) = i.total {
                                i.downloaded = t;
                            }
                        }
                    }
                    if self.inner.lock().unwrap().settings.quarantine {
                        let _ = crate::quarantine::mark(&path, now());
                    }
                    self.set_status(id, Status::Done, None);
                    self.add_history(id);
                    self.webhook("download.completed", id, None);
                    self.maybe_run_after_all();
                    let sound = self.inner.lock().unwrap().settings.sound;
                    let mut note = self.app.notification().builder().title("Download complete").body(name);
                    if sound {
                        note = note.sound("default");
                    }
                    let _ = note.show();
                }
                // Stopped by the schedule: wait in the queue, and start again when the window ends.
                Err(grabnr_core::Error::Cancelled) => self.set_status(id, if held { Status::Queued } else { Status::Paused }, None),
                Err(e) => {
                    let msg = e.to_string();
                    if let Some(delay) = self.schedule_retry(id, &e, &msg) {
                        // Show why it failed while it waits; the status stays Queued so it is not shown as failed.
                        let me = self.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(delay).await;
                            me.pump();
                        });
                        self.pump();
                        return;
                    }
                    self.set_status(id, Status::Error, Some(msg.clone()));
                    self.webhook("download.failed", id, Some(msg.clone()));
                    let _ = self.app.notification().builder().title("Download failed").body(msg).show();
                }
            }
        }
        self.pump();
    }
}

/// Errors worth another try: network trouble, server overload, or a corrupt transfer.
fn is_transient(e: &grabnr_core::Error) -> bool {
    use grabnr_core::Error::*;
    match e {
        Http(_) | AllFailed(_) | ChecksumMismatch { .. } | FileChanged => true,
        Status(s) => *s >= 500 || *s == 429 || *s == 408,
        Io(_) | Db(_) | Cancelled | DiskFull { .. } | Other(_) => false,
    }
}

/// 10 s, 30 s, 90 s … capped at 10 minutes.
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs((10u64 * 3u64.pow(attempt.saturating_sub(1).min(5))).min(600))
}

/// Highest priority first, then oldest; skips anything already running or still waiting out a retry delay.
fn pick_next(
    items: &[Item],
    running: &HashMap<String, CancellationToken>,
    retry_at: &HashMap<String, Instant>,
    now: Instant,
) -> Option<Item> {
    items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.status == Status::Queued && !running.contains_key(&i.id) && retry_at.get(&i.id).is_none_or(|t| *t <= now))
        // items are stored newest first, so a larger index is older
        .max_by_key(|(idx, i)| (i.priority, *idx))
        .map(|(_, i)| i.clone())
}

/// Host and path of a link (the query and fragment are ignored), lowercased where the standard says it does not matter.
fn split_url(u: &str) -> Option<(String, String)> {
    let u = url::Url::parse(u).ok()?;
    Some((u.host_str()?.to_ascii_lowercase(), u.path().to_string()))
}

/// Split `user:password@` out of a link, so it can be kept apart from the link that is shown and stored.
pub fn split_userinfo(link: &str) -> (String, Option<(String, String)>) {
    let Ok(mut u) = url::Url::parse(link) else { return (link.to_string(), None) };
    if u.username().is_empty() && u.password().is_none() {
        return (link.to_string(), None);
    }
    let dec = |s: &str| {
        url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    let creds = (dec(u.username()), dec(u.password().unwrap_or("")));
    let _ = u.set_username("");
    let _ = u.set_password(None);
    (u.to_string(), Some(creds))
}

/// Put credentials back into a link (percent-encoded), for the engine only.
pub fn with_userinfo(link: &str, user: &str, pass: &str) -> String {
    let Ok(mut u) = url::Url::parse(link) else { return link.to_string() };
    let _ = u.set_username(user);
    let _ = u.set_password(if pass.is_empty() { None } else { Some(pass) });
    u.to_string()
}

/// `user:password` from an `Authorization: Basic ...` value.
pub fn basic_credentials(value: &str) -> Option<(String, String)> {
    use base64::Engine as _;
    let b = value.strip_prefix("Basic ")?;
    let raw = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b.trim()).ok()?).ok()?;
    let (u, p) = raw.split_once(':').unwrap_or((raw.as_str(), ""));
    Some((u.to_string(), p.to_string()))
}

/// The command that puts this computer to sleep.
fn sleep_command() -> std::process::Command {
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("pmset");
        c.arg("sleepnow");
        c
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["powrprof.dll,SetSuspendState", "0,1,0"]);
        c
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut c = std::process::Command::new("systemctl");
        c.arg("suspend");
        c
    }
}

/// A command line run through the system shell.
fn shell_command(line: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", line]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", line]);
        c
    }
}

/// ffmpeg, if installed: on the PATH or in the usual Homebrew locations (an app started from the Dock has a short PATH).
pub fn find_ffmpeg() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, priority: i32, status: Status) -> Item {
        Item {
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
            priority,
            retries: 0,
            proxy: None,
            mirrors: vec![],
            quality: None,
            stats: Stats::default(),
        }
    }

    #[test]
    fn oldest_first_unless_prioritised() {
        let items = vec![item("new", 0, Status::Queued), item("old", 0, Status::Queued)];
        let none = HashMap::new();
        assert_eq!(pick_next(&items, &HashMap::new(), &none, Instant::now()).unwrap().id, "old");
        let items = vec![item("new", 1, Status::Queued), item("old", 0, Status::Queued)];
        assert_eq!(pick_next(&items, &HashMap::new(), &none, Instant::now()).unwrap().id, "new");
    }

    #[test]
    fn skips_running_paused_and_waiting_retries() {
        let now = Instant::now();
        let items = vec![item("wait", 5, Status::Queued), item("paused", 9, Status::Paused), item("ok", 0, Status::Queued)];
        let mut retry = HashMap::new();
        retry.insert("wait".to_string(), now + Duration::from_secs(30));
        assert_eq!(pick_next(&items, &HashMap::new(), &retry, now).unwrap().id, "ok");
        assert!(pick_next(&items, &HashMap::new(), &retry, now + Duration::from_secs(31)).unwrap().id == "wait");
    }

    #[test]
    fn a_fresh_link_replaces_an_expired_one_only_when_it_is_clearly_the_same_file() {
        let mut old = item("old", 0, Status::Error);
        old.url = "https://cdn.example/dl/file.zip?sig=old".into();
        old.error = Some("server returned HTTP 403".into());
        old.filename = Some("file.zip".into());
        let items = vec![old.clone()];
        let new = "https://cdn.example/dl/file.zip?sig=new";
        assert_eq!(Manager::relink_target(&items, new, Some("file.zip")).as_deref(), Some("old"));
        assert_eq!(Manager::relink_target(&items, new, None).as_deref(), Some("old"), "no file name given: path match is enough");
        assert!(Manager::relink_target(&items, new, Some("other.zip")).is_none(), "a different file name is a different download");
        assert!(Manager::relink_target(&items, "https://cdn.example/dl/other.zip?sig=new", None).is_none(), "different path");
        assert!(Manager::relink_target(&items, "https://evil.example/dl/file.zip?sig=new", None).is_none(), "different host");
        assert!(Manager::relink_target(&items, &old.url, None).is_none(), "the very same link is just a duplicate");
        let mut paused = old.clone();
        paused.status = Status::Paused;
        assert!(Manager::relink_target(&[paused], new, None).is_none(), "only failed downloads are relinked");
        let mut notfound = old;
        notfound.error = Some("server returned HTTP 404".into());
        assert!(Manager::relink_target(&[notfound], new, None).is_none(), "a 404 is not an expired link");
    }

    #[test]
    fn passwords_are_kept_out_of_the_stored_link() {
        let (clean, creds) = split_userinfo("ftp://me:p%40ss%2Bw@files.example:2121/pub/a.iso");
        assert_eq!(clean, "ftp://files.example:2121/pub/a.iso");
        assert_eq!(creds, Some(("me".to_string(), "p@ss+w".to_string())));
        assert_eq!(split_userinfo("https://x/y"), ("https://x/y".to_string(), None));
        // Round trip into the engine's form and back.
        let back = with_userinfo(&clean, "me", "p@ss+w");
        assert_eq!(split_userinfo(&back).1, creds);
        let header = format!("Basic {}", {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode("me:p@ss+w")
        });
        assert_eq!(basic_credentials(&header), creds);
        assert_eq!(basic_credentials("Bearer abc"), None);
    }

    #[test]
    fn retry_policy() {
        use grabnr_core::Error;
        assert!(is_transient(&Error::AllFailed("x".into())));
        assert!(is_transient(&Error::Status(503)));
        assert!(!is_transient(&Error::Status(404)));
        assert!(!is_transient(&Error::DiskFull { need: 1, free: 0 }));
        assert_eq!(retry_delay(1), Duration::from_secs(10));
        assert_eq!(retry_delay(2), Duration::from_secs(30));
        assert_eq!(retry_delay(20), Duration::from_secs(600));
    }
}
