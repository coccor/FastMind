//! Times a search the way the app runs one: `cargo run --release --example search_timing -- DIR`.
use fastmind_core::{Notebook, remember::LiveIndex, usage::Usage};
use std::time::Instant;

fn main() {
    let nb = Notebook::open(std::env::args().nth(1).expect("notebook folder")).unwrap();
    let mut live = LiveIndex::default();
    let t = Instant::now();
    live.refresh(&nb);
    println!("first refresh: {:?}", t.elapsed());
    let t = Instant::now();
    live.refresh(&nb);
    println!("second refresh (nothing changed): {:?}", t.elapsed());
    let t = Instant::now();
    let usage = Usage::load(&nb);
    println!("usage: {:?}", t.elapsed());
    let index = live.refresh_if_built(&nb);
    for query in ["typing fast", "zzqx", "abc"] {
        let t = Instant::now();
        let hits = index.search(query, 30, &|p| usage.boost(p));
        println!("search {query:?}: {} hits in {:?}", hits.len(), t.elapsed());
    }
}
