mod api;
mod manager;

use std::sync::Arc;

use grabnr_core::{interfaces::shared_gateways, list_links, spike, Link};
use manager::{AddRequest, ItemView, Manager, Settings, SettingsPatch};
use serde::Serialize;
use tauri::{Manager as _, State};
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
fn add_download(m: Mgr, url: String, filename: Option<String>) -> Result<String, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Enter a full http:// or https:// link".into());
    }
    Ok(m.inner().add(AddRequest { url, filename, headers: Vec::new() }))
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
        .setup(|app| {
            let m = Manager::load(app.handle().clone())?;
            api::spawn(m.clone());
            app.manage(m);
            Ok(())
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
            run_spike
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
