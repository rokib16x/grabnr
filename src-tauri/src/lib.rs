mod api;
mod manager;

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

#[derive(Serialize)]
struct AppState {
    downloads: Vec<ItemView>,
    settings: Settings,
    api_port: u16,
    api_ok: bool,
}

#[tauri::command]
fn get_links() -> LinksResponse {
    let links = list_links();
    let shared_gateways = shared_gateways(&links);
    LinksResponse { links, shared_gateways }
}

#[tauri::command]
fn get_state(m: Mgr) -> AppState {
    AppState { downloads: m.list(), settings: m.settings(), api_port: manager::API_PORT, api_ok: *m.api_ok.lock().unwrap() }
}

#[tauri::command]
fn add_download(m: Mgr, url: String, filename: Option<String>, dir: Option<String>, checksum: Option<String>) -> Result<String, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Enter a full http:// or https:// link".into());
    }
    if let Some(c) = checksum.as_deref().filter(|c| !c.trim().is_empty()) {
        grabnr_core::Checksum::parse(c).map_err(|e| e.to_string())?;
    }
    Ok(m.inner().add(AddRequest { url, filename, headers: Vec::new(), dir, checksum }))
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
fn set_settings(m: Mgr, patch: SettingsPatch) -> Settings {
    m.set_settings(patch);
    m.settings()
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

    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last = String::new();
        loop {
            tick.tick().await;
            let (n, bps) = m.summary();
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
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .setup(|app| {
            let m = Manager::load(app.handle().clone())?;
            api::spawn(m.clone());
            build_tray(app.handle(), m.clone())?;
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
