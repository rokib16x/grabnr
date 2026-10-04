use grabnr_core::{interfaces::shared_gateways, list_links, spike, Link};
use serde::Serialize;

#[derive(Serialize)]
struct LinksResponse {
    links: Vec<Link>,
    shared_gateways: Vec<(String, String)>,
}

#[tauri::command]
fn get_links() -> LinksResponse {
    let links = list_links();
    let shared_gateways = shared_gateways(&links);
    LinksResponse { links, shared_gateways }
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
        .invoke_handler(tauri::generate_handler![get_links, run_spike])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
