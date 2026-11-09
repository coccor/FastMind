//! `fastmind serve`: the desktop app's UI and command surface over HTTP on localhost, for a
//! browser, a headless machine, or testing the UI without the desktop shell.
//! Binds to 127.0.0.1 only: the notebook is never exposed to the network.

use fastmind_core::api::App;
use serde_json::{Value, json};
use std::path::PathBuf;
use tiny_http::{Header, Method, Response, Server};

pub fn serve(app: &App, port: u16, ui: Option<PathBuf>) -> Result<(), String> {
    let ui = ui
        .or_else(find_ui)
        .ok_or("couldn't find the UI folder; pass --ui path/to/apps/desktop/ui")?;
    let server = Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    println!("FastMind is at http://127.0.0.1:{port}/  (Ctrl+C stops it)");
    for mut request in server.incoming_requests() {
        let url = request.url().split('?').next().unwrap_or("/").to_owned();
        let response =
            if let (Method::Post, Some(command)) = (request.method(), url.strip_prefix("/api/")) {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let args: Value = serde_json::from_str(&body).unwrap_or(json!({}));
                let (status, value) = match app.call(command, args) {
                    Ok(value) => (200, json!({ "ok": value })),
                    Err(error) => (400, json!({ "error": error.0 })),
                };
                Response::from_string(value.to_string())
                    .with_status_code(status)
                    .with_header(header("Content-Type", "application/json"))
            } else {
                let rel = if url == "/" {
                    "index.html".to_owned()
                } else {
                    url.trim_start_matches('/').to_owned()
                };
                if rel.contains("..") {
                    Response::from_string("not found").with_status_code(404)
                } else {
                    match std::fs::read(ui.join(&rel)) {
                        Ok(bytes) => Response::from_data(bytes)
                            .with_header(header("Content-Type", content_type(&rel))),
                        Err(_) => Response::from_string("not found").with_status_code(404),
                    }
                }
            };
        let _ = request.respond(response);
    }
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// The UI next to the executable (installed) or in the source tree (development).
fn find_ui() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut candidates = vec![exe.with_file_name("ui")];
    for ancestor in exe.ancestors() {
        candidates.push(ancestor.join("apps").join("desktop").join("ui"));
    }
    candidates.push(PathBuf::from("apps/desktop/ui"));
    candidates
        .into_iter()
        .find(|path| path.join("index.html").exists())
}
