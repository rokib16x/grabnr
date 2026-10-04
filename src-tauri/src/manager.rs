//! Owns the download list: persistence, queueing, pause/resume, and forwarding
//! engine events to the UI. Shared by the Tauri commands and the browser-extension API.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use grabnr_core::{interfaces::LinkKind, list_links, Event, Options, Route, Store};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

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
}

#[derive(Deserialize)]
pub struct SettingsPatch {
    pub dest_dir: Option<String>,
    pub enabled_links: Option<Option<Vec<String>>>,
    pub conns_per_route: Option<usize>,
    pub max_active: Option<usize>,
}

pub struct AddRequest {
    pub url: String,
    pub filename: Option<String>,
    pub headers: Vec<(String, String)>,
}

struct Inner {
    items: Vec<Item>,
    settings: Settings,
    cancels: HashMap<String, CancellationToken>,
    /// Ids being removed, so a cancel is not reported as "paused".
    removing: Vec<String>,
}

pub struct Manager {
    app: AppHandle,
    data_dir: PathBuf,
    store: Arc<Store>,
    inner: Mutex<Inner>,
    pairing_until: Mutex<Option<Instant>>,
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
            });
        let mut items: Vec<Item> =
            std::fs::read(data_dir.join("downloads.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // Anything that was running when the app quit is resumable, not running.
        for i in &mut items {
            if matches!(i.status, Status::Downloading | Status::Queued) {
                i.status = Status::Paused;
            }
        }
        let m = Arc::new(Manager {
            app,
            data_dir,
            store,
            inner: Mutex::new(Inner { items, settings, cancels: HashMap::new(), removing: Vec::new() }),
            pairing_until: Mutex::new(None),
            api_ok: Mutex::new(false),
        });
        m.persist();
        Ok(m)
    }

    fn persist(&self) {
        let g = self.inner.lock().unwrap();
        if let Ok(b) = serde_json::to_vec_pretty(&g.items) {
            let _ = write_private(&self.data_dir.join("downloads.json"), &b);
        }
        if let Ok(b) = serde_json::to_vec_pretty(&g.settings) {
            let _ = write_private(&self.data_dir.join("settings.json"), &b);
        }
    }

    pub fn list(&self) -> Vec<ItemView> {
        self.inner.lock().unwrap().items.iter().map(ItemView::from).collect()
    }

    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, p: SettingsPatch) {
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
        }
        self.persist();
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

    pub fn add(self: &Arc<Self>, req: AddRequest) -> String {
        let id = random_hex(8);
        {
            let mut g = self.inner.lock().unwrap();
            let dir = g.settings.dest_dir.clone();
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
                },
            );
        }
        self.persist();
        self.emit_item(&id);
        self.pump();
        id
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
                if active >= g.settings.max_active {
                    None
                } else {
                    g.items.iter().rev().find(|i| i.status == Status::Queued && !g.cancels.contains_key(&i.id)).cloned()
                }
            };
            let Some(item) = next else { return };
            self.start(item);
        }
    }

    fn routes(&self) -> Vec<Route> {
        let enabled = self.inner.lock().unwrap().settings.enabled_links.clone();
        let routes: Vec<Route> = list_links()
            .iter()
            .filter(|l| match &enabled {
                Some(names) => names.contains(&l.name),
                None => l.kind != LinkKind::Tunnel,
            })
            .map(Route::from_link)
            .collect();
        if routes.is_empty() {
            vec![Route::unbound("default")]
        } else {
            routes
        }
    }

    fn start(self: &Arc<Self>, item: Item) {
        let (conns, cancel) = {
            let mut g = self.inner.lock().unwrap();
            let c = CancellationToken::new();
            g.cancels.insert(item.id.clone(), c.clone());
            (g.settings.conns_per_route, c)
        };
        self.set_status(&item.id, Status::Downloading, None);

        let mut opts = Options::new(item.url.clone(), &item.dir, self.routes());
        opts.filename = item.filename.clone();
        opts.headers = item.headers.clone();
        opts.conns_per_route = conns;
        opts.store = Some(self.store.clone());

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
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                match &e {
                    Event::Started { filename, total, .. } => {
                        i.filename.get_or_insert_with(|| filename.clone());
                        i.total = *total;
                        changed = true;
                    }
                    Event::Progress(s) => i.downloaded = s.downloaded,
                    _ => {}
                }
            }
        }
        if changed {
            self.emit_item(id);
        }
        let _ = self.app.emit("download-event", serde_json::json!({ "id": id, "event": e }));
    }

    fn finish(self: &Arc<Self>, id: &str, res: grabnr_core::Result<PathBuf>) {
        let removed = {
            let mut g = self.inner.lock().unwrap();
            g.cancels.remove(id);
            g.removing.iter().position(|r| r == id).map(|p| g.removing.remove(p)).is_some()
        };
        if !removed {
            match res {
                Ok(path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    {
                        let mut g = self.inner.lock().unwrap();
                        if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                            i.path = Some(path.display().to_string());
                            i.filename = Some(name.clone());
                            if let Some(t) = i.total {
                                i.downloaded = t;
                            }
                        }
                    }
                    self.set_status(id, Status::Done, None);
                    let _ = self.app.notification().builder().title("Download complete").body(name).show();
                }
                Err(grabnr_core::Error::Cancelled) => self.set_status(id, Status::Paused, None),
                Err(e) => {
                    let msg = e.to_string();
                    self.set_status(id, Status::Error, Some(msg.clone()));
                    let _ = self.app.notification().builder().title("Download failed").body(msg).show();
                }
            }
        }
        self.pump();
    }
}
