//! Showing `.fdraw` diagrams through the command surface the desktop app uses.

use fastmind_core::api::App;
use fastmind_core::config::Settings;
use serde_json::json;
use std::fs;

fn app_in(dir: &std::path::Path) -> App {
    let settings = Settings {
        notebook: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    App::ephemeral(settings).offline_for_tests()
}

fn put(dir: &std::path::Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const PREFIX: &str = "data:image/svg+xml;base64,";
const GOOD: &str = "a: focal \"A\" at 40,40
b: store \"B\" at 300,40
a -> b \"x\"";

#[test]
fn image_draws_fdraw() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "notes/attachments/d.fdraw", GOOD);
    let app = app_in(dir.path());
    let got = app
        .call("image", json!({"path": "notes/attachments/d.fdraw"}))
        .unwrap();
    assert!(got["data"].as_str().unwrap().starts_with(PREFIX), "{got}");
}

#[test]
fn image_reports_diagram_errors() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "notes/attachments/bad.fdraw",
        "@@@ nonsense (((\n",
    );
    let app = app_in(dir.path());
    let err = app
        .call("image", json!({"path": "notes/attachments/bad.fdraw"}))
        .unwrap_err();
    assert!(err.to_string().contains("Line 1:"), "{err}");
}

#[test]
fn diagram_draw_draws_text() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let got = app
        .call("diagram_draw", json!({"text": GOOD, "dark": true}))
        .unwrap();
    assert!(got["data"].as_str().unwrap().starts_with(PREFIX), "{got}");
    let err = app
        .call("diagram_draw", json!({"text": "@@@ nonsense (((\n"}))
        .unwrap_err();
    assert!(err.to_string().contains("Line 1:"), "{err}");
}
