mod api;
mod deeplink;
mod history;
mod manager;
mod portable;
mod quarantine;
mod queue;
mod report;
mod schedule;
mod secrets;
mod thumbs;

use std::sync::Arc;

use grabnr_core::{interfaces::shared_gateways, list_links, spike, Link};
use manager::{AddRequest, ItemView, Manager, Settings, SettingsPatch};
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter as _, Manager as _, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_opener::OpenerExt;

type Mgr<'a> = State<'a, Arc<Manager>>;

#[derive(Serialize)]
struct LinksResponse {
    links: Vec<Link>,
    shared_gateways: Vec<(String, String)>,
}

/// Optional extras for one download. Credentials are stored with the download and never sent back to the UI.
#[derive(serde::Deserialize, Default, Clone)]
struct AddOptions {
    filename: Option<String>,
    dir: Option<String>,
    checksum: Option<String>,
    username: Option<String>,
    password: Option<String>,
    token: Option<String>,
    proxy: Option<String>,
    mirrors: Option<Vec<String>>,
    quality: Option<String>,
}

#[derive(Serialize)]
struct AddResult {
    id: String,
    /// The link was already in the list, so nothing new was added.
    duplicate: bool,
}

#[derive(Serialize)]
struct AppState {
    downloads: Vec<ItemView>,
    settings: Settings,
    api_port: u16,
    api_ok: bool,
    schedule: schedule::Effect,
    after_all: manager::AfterAll,
    keychain_available: bool,
}

#[tauri::command]
fn get_links() -> LinksResponse {
    let links = list_links();
    let shared_gateways = shared_gateways(&links);
    LinksResponse { links, shared_gateways }
}

#[tauri::command]
fn get_state(m: Mgr) -> AppState {
    AppState {
        downloads: m.list(),
        settings: m.settings(),
        api_port: manager::API_PORT,
        api_ok: *m.api_ok.lock().unwrap(),
        schedule: m.schedule_effect(),
        after_all: m.after_all(),
        keychain_available: m.keychain_available(),
    }
}

/// Turn the form's extras into an `AddRequest` for one URL.
fn build_request(url: String, o: &AddOptions) -> Result<AddRequest, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Enter a full http:// or https:// link".into());
    }
    if let Some(c) = o.checksum.as_deref().filter(|c| !c.trim().is_empty()) {
        grabnr_core::Checksum::parse(c).map_err(|e| e.to_string())?;
    }
    let proxy = o.proxy.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &proxy {
        if !["http://", "https://", "socks5://", "socks5h://"].iter().any(|s| p.starts_with(s)) {
            return Err("The proxy must start with http://, https://, socks5:// or socks5h://".into());
        }
    }
    let mut headers = Vec::new();
    if let Some(t) = o.token.as_ref().filter(|t| !t.trim().is_empty()) {
        headers.push(("Authorization".to_string(), grabnr_core::Auth::Bearer(t.trim().to_string()).header_value()));
    } else if let Some(user) = o.username.as_ref().filter(|u| !u.is_empty()) {
        headers.push((
            "Authorization".to_string(),
            grabnr_core::Auth::Basic { user: user.clone(), pass: o.password.clone().unwrap_or_default() }.header_value(),
        ));
    }
    let mirrors = o.mirrors.as_deref().map(grabnr_core::links::parse_url_list_from_vec).unwrap_or_default();
    Ok(AddRequest {
        url,
        filename: o.filename.clone(),
        headers,
        dir: o.dir.clone(),
        checksum: o.checksum.clone(),
        proxy,
        mirrors,
        quality: o.quality.clone(),
    })
}

/// A Metalink file stands for one download with several mirrors, a size and a hash.
async fn expand_metalink(mut req: AddRequest) -> Result<AddRequest, String> {
    if !grabnr_core::metalink::looks_like(&req.url, None) {
        return Ok(req);
    }
    let (text, _) =
        grabnr_core::fetch::fetch_text(&req.url, &req.headers, req.proxy.as_deref(), 2 << 20).await.map_err(|e| e.to_string())?;
    let m = grabnr_core::metalink::parse(&text).map_err(|e| e.to_string())?;
    req.url = m.urls[0].clone();
    req.mirrors.extend(m.urls.iter().skip(1).cloned());
    if req.filename.as_deref().is_none_or(str::is_empty) && !m.name.is_empty() {
        req.filename = Some(m.name.clone());
    }
    if req.checksum.as_deref().is_none_or(|c| c.trim().is_empty()) {
        req.checksum = m.checksum().map(|c| c.spec());
    }
    Ok(req)
}

#[tauri::command]
async fn add_download(m: Mgr<'_>, url: String, options: Option<AddOptions>) -> Result<AddResult, String> {
    let req = expand_metalink(build_request(url, &options.unwrap_or_default())?).await?;
    let (id, duplicate) = m.inner().add(req);
    Ok(AddResult { id, duplicate })
}

#[derive(Serialize)]
struct BatchResult {
    added: usize,
    duplicates: usize,
    /// Links that could not be added (for example an unreadable Metalink file).
    skipped: usize,
}

/// Add every link in a pasted list (one per line).
#[tauri::command]
async fn add_batch(m: Mgr<'_>, text: String, options: Option<AddOptions>) -> Result<BatchResult, String> {
    let urls = grabnr_core::links::parse_url_list(&text);
    if urls.is_empty() {
        return Err("No http or https links found".into());
    }
    let o = options.unwrap_or_default();
    let (mut added, mut duplicates, mut skipped) = (0, 0, 0);
    for u in urls {
        // A batch ignores per-file names, hashes and mirrors: they belong to one file.
        let one = AddOptions { filename: None, checksum: None, mirrors: None, ..o.clone() };
        let Ok(req) = expand_metalink(build_request(u, &one)?).await else {
            skipped += 1;
            continue;
        };
        if m.inner().add(req).1 {
            duplicates += 1;
        } else {
            added += 1;
        }
    }
    Ok(BatchResult { added, duplicates, skipped })
}

/// The qualities a streaming (HLS) link offers, best first. Empty when it has only one.
#[tauri::command]
async fn list_hls_variants(m: Mgr<'_>, url: String) -> Result<Vec<grabnr_core::hls::Variant>, String> {
    let proxy = Some(m.settings().proxy).filter(|p| !p.is_empty());
    let (text, _) = grabnr_core::fetch::fetch_text(url.trim(), &[], proxy.as_deref(), 2 << 20).await.map_err(|e| e.to_string())?;
    match grabnr_core::hls::parse(&text, url.trim()).map_err(|e| e.to_string())? {
        grabnr_core::hls::Playlist::Master(mut v) => {
            v.sort_by_key(|x| std::cmp::Reverse((x.height.unwrap_or(0), x.bandwidth)));
            Ok(v)
        }
        grabnr_core::hls::Playlist::Media(_) => Ok(Vec::new()),
    }
}

/// Links found on a web page, for picking which files to download.
#[tauri::command]
async fn grab_links(m: Mgr<'_>, url: String) -> Result<Vec<grabnr_core::links::PageLink>, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Enter a full http:// or https:// link".into());
    }
    let proxy = Some(m.settings().proxy).filter(|p| !p.is_empty());
    let (html, _) = grabnr_core::fetch::fetch_text(&url, &[], proxy.as_deref(), 5 << 20).await.map_err(|e| e.to_string())?;
    let links = grabnr_core::links::extract_links(&html, &url);
    if links.is_empty() {
        return Err("No links found on that page".into());
    }
    Ok(links)
}

#[tauri::command]
fn get_history(m: Mgr) -> Vec<history::HistoryRec> {
    m.history()
}

#[tauri::command]
fn clear_history(m: Mgr) {
    m.clear_history();
}

/// Write the history as CSV to a path the user picked in a save dialog.
#[tauri::command]
fn export_history(m: Mgr, path: String) -> Result<(), String> {
    std::fs::write(path, m.history_csv()).map_err(|e| e.to_string())
}

/// Save settings (without the token or proxy credentials) to a file the user picked.
#[tauri::command]
fn export_settings(m: Mgr, path: String) -> Result<(), String> {
    let v = portable::export_settings(&m.settings());
    std::fs::write(path, serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_settings(m: Mgr, path: String) -> Result<Settings, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    m.set_settings(portable::import_settings(&text)?);
    Ok(m.settings())
}

/// Save the links still to download, one per line.
#[tauri::command]
fn export_queue(m: Mgr, path: String) -> Result<(), String> {
    std::fs::write(path, m.queue_links()).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_diagnostics(m: Mgr) -> String {
    m.diagnostics(&list_links())
}

/// A preview image of a finished file as a data URL, or nothing when none can be made.
#[tauri::command]
async fn thumbnail(m: Mgr<'_>, id: String) -> Result<Option<String>, String> {
    use base64::Engine as _;
    let Some((file, cache)) = m.thumbnail_source(&id) else { return Ok(None) };
    let bytes = tauri::async_runtime::spawn_blocking(move || thumbs::make(&file, &cache, &id)).await.map_err(|e| e.to_string())?;
    Ok(bytes.map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b))))
}

#[tauri::command]
fn update_link(m: Mgr, id: String, url: String) -> Result<(), String> {
    m.inner().update_link(&id, &url)
}

#[tauri::command]
fn reorder_download(m: Mgr, id: String, before: Option<String>) {
    m.reorder(&id, before.as_deref());
}

#[tauri::command]
fn set_after_all(m: Mgr, action: manager::AfterAll) {
    m.set_after_all(action);
}

#[tauri::command]
fn move_download(m: Mgr, id: String, to_front: bool) {
    m.inner().move_item(&id, to_front);
}

#[tauri::command]
fn pause_download(m: Mgr, id: String) {
    m.pause(&id);
}

#[tauri::command]
fn resume_download(m: Mgr, id: String) {
    m.inner().resume(&id);
}

#[tauri::command]
fn remove_download(m: Mgr, id: String, delete_files: bool) {
    m.remove(&id, delete_files);
}

#[tauri::command]
fn reveal_download(app: tauri::AppHandle, m: Mgr, id: String) -> Result<(), String> {
    let p = m.find_path(&id).ok_or("unknown download")?;
    app.opener().reveal_item_in_dir(p).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_download(app: tauri::AppHandle, m: Mgr, id: String) -> Result<(), String> {
    let p = m.find_path(&id).ok_or("unknown download")?;
    app.opener().open_path(p.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_settings(m: Mgr, patch: SettingsPatch) -> Result<Settings, String> {
    if patch.keychain == Some(true) {
        m.keychain_probe()?;
    }
    m.set_settings(patch);
    Ok(m.settings())
}

#[tauri::command]
fn allow_pairing(m: Mgr) -> u64 {
    m.allow_pairing(60);
    60
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let a = app.autolaunch();
    if enabled { a.enable() } else { a.disable() }.map_err(|e| e.to_string())?;
    Ok(a.is_enabled().unwrap_or(enabled))
}

/// Add the downloads named by `grabnr://` links and bring the window forward.
fn open_links(app: &tauri::AppHandle, m: &Arc<Manager>, addresses: Vec<String>) {
    let links: Vec<String> = addresses.iter().flat_map(|a| deeplink::links_from(a)).collect();
    if links.is_empty() {
        return;
    }
    let (app, m) = (app.clone(), m.clone());
    tauri::async_runtime::spawn(async move {
        let mut added = 0;
        for u in links {
            let Ok(req) = build_request(u, &AddOptions::default()) else { continue };
            if let Ok(req) = expand_metalink(req).await {
                if !m.add(req).1 {
                    added += 1;
                }
            }
        }
        if added > 0 {
            let _ = tauri_plugin_notification::NotificationExt::notification(&app)
                .builder()
                .title("Added from a link")
                .body(format!("{added} download(s) added"))
                .show();
        }
        show_main(&app);
    });
}

#[tauri::command]
fn pause_all(m: Mgr) {
    m.pause_all();
}

#[tauri::command]
fn resume_all(m: Mgr) {
    m.inner().resume_all();
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// From the menu bar popover: bring the main window forward, optionally on the settings sheet.
#[tauri::command]
fn show_main_window(app: tauri::AppHandle, settings: bool) {
    hide_popover(&app);
    show_main(&app);
    if settings {
        let _ = app.emit_to("main", "open-settings", ());
    }
}

fn hide_popover(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.hide();
    }
}

/// Show the popover centred under the menu bar icon, kept on the screen it was clicked on.
fn toggle_popover(app: &tauri::AppHandle, icon: tauri::Rect) {
    let Some(w) = app.get_webview_window("tray") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    let scale = w.scale_factor().unwrap_or(1.0);
    let pos = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let width = w.outer_size().map(|s| s.width as f64).unwrap_or(340.0 * scale);
    let mut x = pos.x + size.width / 2.0 - width / 2.0;
    if let Ok(Some(mon)) = app.monitor_from_point(pos.x, pos.y) {
        let (left, right) = (mon.position().x as f64, mon.position().x as f64 + mon.size().width as f64);
        x = x.clamp(left + 8.0 * scale, right - width - 8.0 * scale);
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(x, pos.y + size.height + 6.0 * scale));
    let _ = w.show();
    let _ = w.set_focus();
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn human_rate(bps: f64) -> String {
    let (mut v, mut u) = (bps, 0);
    while v >= 1024.0 && u < 3 {
        v /= 1024.0;
        u += 1;
    }
    format!("{:.1} {}/s", v, ["B", "KB", "MB", "GB"][u])
}

/// Menu bar icon: live speed, show/pause/resume/quit. The app keeps running when the window is closed.
fn build_tray(app: &tauri::AppHandle, m: Arc<Manager>) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Idle", false, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "show", "Open grabnr", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_all", "Pause all", true, None::<&str>)?,
            &MenuItem::with_id(app, "resume_all", "Resume all", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit grabnr", true, None::<&str>)?,
        ],
    )?;
    let mm = m.clone();
    let tray = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("grabnr")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = ev {
                toggle_popover(tray.app_handle(), rect);
            }
        })
        .on_menu_event(move |app, ev| match ev.id.as_ref() {
            "show" => show_main(app),
            "pause_all" => mm.pause_all(),
            "resume_all" => mm.resume_all(),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last = String::new();
        loop {
            tick.tick().await;
            let (n, bps) = m.summary();
            dock_progress(&app_handle, n, m.overall_progress());
            let text = if n == 0 { "Idle".to_string() } else { format!("{n} downloading · {}", human_rate(bps)) };
            if text != last {
                let _ = status.set_text(&text);
                // macOS shows this text next to the icon; clear it when idle.
                let _ = tray.set_title(if n == 0 { None } else { Some(human_rate(bps)) });
                let _ = tray.set_tooltip(Some(&format!("grabnr: {text}")));
                last = text;
            }
        }
    });
    Ok(())
}

/// Dock icon: a progress bar and the number of running downloads.
fn dock_progress(app: &tauri::AppHandle, running: usize, pct: Option<u64>) {
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    let Some(w) = app.get_webview_window("main") else { return };
    let state = match (running, pct) {
        (0, _) => ProgressBarState { status: Some(ProgressBarStatus::None), progress: None },
        (_, Some(p)) => ProgressBarState { status: Some(ProgressBarStatus::Normal), progress: Some(p.min(100)) },
        (_, None) => ProgressBarState { status: Some(ProgressBarStatus::Indeterminate), progress: None },
    };
    let _ = w.set_progress_bar(state);
    // The Dock badge exists on macOS only.
    #[cfg(target_os = "macos")]
    let _ = w.set_badge_label(if running == 0 { None } else { Some(running.to_string()) });
}

#[tauri::command]
async fn run_spike(only: Vec<String>, secs: u64) -> Result<spike::SpikeReport, String> {
    let mut links = list_links();
    if !only.is_empty() {
        links.retain(|l| only.contains(&l.name));
    }
    if links.is_empty() {
        return Err("no usable links selected".into());
    }
    Ok(spike::run(&links, spike::DEFAULT_TEST_URL, secs.clamp(2, 30)).await)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .setup(|app| {
            let m = Manager::load(app.handle().clone())?;
            api::spawn(m.clone());
            build_tray(app.handle(), m.clone())?;
            // Re-evaluate the schedule every 20 seconds (it is minute resolution).
            let sched = m.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
                loop {
                    tick.tick().await;
                    sched.apply_schedule_now();
                }
            });
            // grabnr://add?url=… hands downloads to grabnr from a web page or another app.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let (handle, mgr) = (app.handle().clone(), m.clone());
                app.deep_link().on_open_url(move |e| open_links(&handle, &mgr, e.urls().iter().map(|u| u.to_string()).collect()));
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    open_links(app.handle(), &m, urls.iter().map(|u| u.to_string()).collect());
                }
            }
            app.manage(m);
            // Started by the login item: stay in the menu bar until the user opens the window.
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        // Closing the window hides it; downloads and the browser-extension API keep running.
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // The menu bar popover behaves like a menu: clicking elsewhere dismisses it.
                WindowEvent::Focused(false) if window.label() == "tray" => {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_links,
            get_state,
            add_download,
            add_batch,
            grab_links,
            list_hls_variants,
            get_history,
            clear_history,
            export_history,
            export_settings,
            import_settings,
            export_queue,
            get_diagnostics,
            thumbnail,
            update_link,
            reorder_download,
            set_after_all,
            move_download,
            pause_download,
            resume_download,
            remove_download,
            reveal_download,
            open_download,
            set_settings,
            allow_pairing,
            pause_all,
            resume_all,
            quit_app,
            show_main_window,
            get_autostart,
            set_autostart,
            run_spike
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Clicking the Dock icon with no visible window brings it back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
