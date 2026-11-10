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
        let response = if url.starts_with("/api/") && !from_this_page(&request, port) {
            // Any web page can send a POST to localhost, and a hostile one could rename the
            // server's host to its own (DNS rebinding): only this server's own page may call.
            Response::from_string("forbidden").with_status_code(403)
        } else if let (Method::Post, Some(command)) = (request.method(), url.strip_prefix("/api/"))
        {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let args: Value = serde_json::from_str(&body).unwrap_or(json!({}));
            let result = if command == "os_open" {
                open_in_editor(app, &args)
            } else {
                app.call(command, args)
            };
            let (status, value) = match result {
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

/// Opens a note in the system's default editor. Only files inside the notebook, so a page can't
/// ask the server to launch anything else.
fn open_in_editor(app: &App, args: &Value) -> fastmind_core::Result<Value> {
    let path = std::path::PathBuf::from(args["path"].as_str().unwrap_or_default());
    let notebook = app.notebook()?;
    let path = path
        .canonicalize()
        .map_err(|e| fastmind_core::Error::msg(e.to_string()))?;
    let root = notebook
        .root
        .canonicalize()
        .map_err(|e| fastmind_core::Error::msg(e.to_string()))?;
    if !path.starts_with(&root) {
        return Err(fastmind_core::Error::msg(
            "only notes inside the notebook can be opened",
        ));
    }
    opener::open(&path).map_err(|e| fastmind_core::Error::msg(e.to_string()))?;
    Ok(json!({ "opened": path }))
}

/// True when a request comes from the page this server serves: its Host is this server, and a
/// browser's Origin, when it sends one, is too.
fn from_this_page(request: &tiny_http::Request, port: u16) -> bool {
    let value = |name: &str| {
        request
            .headers()
            .iter()
            .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str().to_owned())
    };
    allowed(value("Host").as_deref(), value("Origin").as_deref(), port)
}

fn allowed(host: Option<&str>, origin: Option<&str>, port: u16) -> bool {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host_ok = host.is_some_and(|h| hosts.iter().any(|ok| h.eq_ignore_ascii_case(ok)));
    let origin_ok = origin.is_none_or(|o| {
        hosts
            .iter()
            .any(|ok| o.eq_ignore_ascii_case(&format!("http://{ok}")))
    });
    host_ok && origin_ok
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

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn only_this_servers_page_may_call_the_api() {
        assert!(allowed(
            Some("127.0.0.1:8765"),
            Some("http://127.0.0.1:8765"),
            8765
        ));
        assert!(allowed(Some("localhost:8765"), None, 8765));
        // Another site's page posting to localhost.
        assert!(!allowed(
            Some("127.0.0.1:8765"),
            Some("https://evil.example"),
            8765
        ));
        assert!(!allowed(Some("127.0.0.1:8765"), Some("null"), 8765));
        // DNS rebinding: the browser thinks it's talking to evil.example.
        assert!(!allowed(
            Some("evil.example:8765"),
            Some("http://evil.example:8765"),
            8765
        ));
        assert!(!allowed(None, None, 8765));
        assert!(!allowed(Some("127.0.0.1:9999"), None, 8765));
    }
}
