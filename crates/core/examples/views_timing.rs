//! Times the commands behind the app's views: `cargo run --release --example views_timing -- DIR`.
use fastmind_core::{api::App, config::Settings};
use serde_json::json;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("notebook folder");
    let app = App::ephemeral(Settings {
        notebook: Some(dir.into()),
        ..Settings::default()
    });
    for command in ["status", "inbox", "home", "status"] {
        let t = Instant::now();
        app.call(command, json!({})).unwrap();
        println!("{command}: {:?}", t.elapsed());
    }
}
