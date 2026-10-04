//! Local HTTP API for the browser extension. Listens on 127.0.0.1 only.
//!
//! - `GET /ping`  no auth: lets the extension detect that the app is running
//! - `GET /pair`  returns the token, but only while the user has opened a pairing window in the app
//! - `POST /add`  `Authorization: Bearer <token>`, JSON `{url, filename?, referrer?, cookies?, userAgent?, headers?}`
//!
//! Requests that carry a web-page `Origin` are refused, so a website cannot talk to this port.
//!
//! The official grabnr extension has a fixed ID (its manifest pins a public key). Browsers set the
//! `Origin` header themselves and pages cannot forge it, so requests from that exact extension are
//! trusted without a token: no pairing step. Any other extension still needs the token.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::manager::{AddRequest, Manager, API_PORT};

#[derive(Deserialize)]
struct AddBody {
    url: String,
    filename: Option<String>,
    referrer: Option<String>,
    cookies: Option<String>,
    #[serde(rename = "userAgent")]
    user_agent: Option<String>,
    headers: Option<HashMap<String, String>>,
}

pub fn spawn(m: Arc<Manager>) {
    let server = match Server::http(("127.0.0.1", API_PORT)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("grabnr: browser API could not start on port {API_PORT}: {e}");
            return;
        }
    };
    *m.api_ok.lock().unwrap() = true;
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            handle(&m, req);
        }
    });
}

fn header(req: &Request, name: &str) -> Option<String> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

/// Origins of the official extension build, trusted without a token.
const TRUSTED_ORIGINS: [&str; 1] = ["chrome-extension://nmfkcamnjeiknlpdpepoomkalaglenbb"];

fn extension_origin(o: &str) -> bool {
    ["chrome-extension://", "moz-extension://", "safari-web-extension://"].iter().any(|p| o.starts_with(p))
}

fn reply(req: Request, status: u16, body: serde_json::Value, origin: Option<&str>) {
    let mut r = Response::from_string(body.to_string()).with_status_code(status);
    r.add_header(Header::from_bytes("Content-Type", "application/json").unwrap());
    if let Some(o) = origin {
        r.add_header(Header::from_bytes("Access-Control-Allow-Origin", o).unwrap());
        r.add_header(Header::from_bytes("Access-Control-Allow-Headers", "authorization, content-type").unwrap());
        r.add_header(Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS").unwrap());
    }
    let _ = req.respond(r);
}

/// Headers the engine or HTTP stack own; a client must not be able to override them.
const BLOCKED: [&str; 8] = ["host", "content-length", "range", "if-range", "connection", "transfer-encoding", "te", "upgrade"];

fn handle(m: &Arc<Manager>, mut req: Request) {
    let origin = header(&req, "origin");
    let host_ok = header(&req, "host").is_some_and(|h| h == format!("127.0.0.1:{}", crate::manager::API_PORT) || h == format!("localhost:{}", crate::manager::API_PORT));
    // Host check blocks DNS-rebinding; Origin check blocks web pages.
    if !host_ok || origin.as_deref().is_some_and(|o| !extension_origin(o)) {
        return reply(req, 403, json!({"error": "forbidden"}), None);
    }
    let cors = origin.as_deref();
    let path = req.url().split('?').next().unwrap_or("").to_string();

    match (req.method().clone(), path.as_str()) {
        (Method::Options, _) => reply(req, 204, json!(null), cors),
        (Method::Get, "/ping") => reply(req, 200, json!({"app": "grabnr", "version": env!("CARGO_PKG_VERSION"), "paired": m.paired()}), cors),
        (Method::Get, "/pair") => match m.pairing_token() {
            Some(t) => reply(req, 200, json!({"token": t}), cors),
            None => reply(req, 403, json!({"error": "pairing_closed"}), cors),
        },
        (Method::Post, "/add") => {
            let token = header(&req, "authorization").and_then(|a| a.strip_prefix("Bearer ").map(str::to_owned)).unwrap_or_default();
            let trusted = origin.as_deref().is_some_and(|o| TRUSTED_ORIGINS.contains(&o));
            if !trusted && !m.token_ok(&token) {
                return reply(req, 401, json!({"error": "unauthorized"}), cors);
            }
            let mut body = String::new();
            if req.as_reader().take(64 * 1024).read_to_string(&mut body).is_err() {
                return reply(req, 400, json!({"error": "unreadable body"}), cors);
            }
            let Ok(b) = serde_json::from_str::<AddBody>(&body) else {
                return reply(req, 400, json!({"error": "expected JSON with a url"}), cors);
            };
            if !(b.url.starts_with("http://") || b.url.starts_with("https://")) {
                return reply(req, 400, json!({"error": "only http and https URLs are supported"}), cors);
            }
            let mut headers: Vec<(String, String)> = Vec::new();
            for (k, v) in b.headers.unwrap_or_default() {
                if !BLOCKED.contains(&k.to_ascii_lowercase().as_str()) {
                    headers.push((k, v));
                }
            }
            for (name, val) in [("Referer", b.referrer), ("Cookie", b.cookies), ("User-Agent", b.user_agent)] {
                if let Some(v) = val.filter(|v| !v.is_empty()) {
                    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
                    headers.push((name.into(), v));
                }
            }
            let (id, duplicate) = m.add(AddRequest { url: b.url, filename: b.filename, headers, dir: None, checksum: None, proxy: None, mirrors: Vec::new() });
            reply(req, 200, json!({"id": id, "duplicate": duplicate}), cors)
        }
        _ => reply(req, 404, json!({"error": "not found"}), cors),
    }
}
