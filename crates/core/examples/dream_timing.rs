//! Times each stage Dream runs on a notebook: `cargo run --release --example dream_timing -- DIR`.
use fastmind_core::{Notebook, dream::analyze, links::Resolver, notebook};
use std::time::Instant;

fn main() {
    let root = std::env::args().nth(1).expect("notebook folder");
    let nb = Notebook::open(root).unwrap();
    let t = Instant::now();
    let notes = nb.notes();
    println!("read {} notes: {:?}", notes.len(), t.elapsed());
    let t = Instant::now();
    let _ = notebook::backlinks(&notes);
    println!("backlinks: {:?}", t.elapsed());
    let t = Instant::now();
    let features = analyze::features(&notes);
    println!("features: {:?}", t.elapsed());
    let t = Instant::now();
    let allowed = vec![true; notes.len()];
    let pairs = analyze::candidate_pairs(&features, &allowed);
    println!("{} candidate pairs: {:?}", pairs.len(), t.elapsed());
    let t = Instant::now();
    let dups = pairs
        .iter()
        .filter(|(a, b)| analyze::duplicates(&features[*a], &features[*b], 0.55).is_some())
        .count();
    println!("{dups} duplicate pairs: {:?}", t.elapsed());
    let t = Instant::now();
    let resolver = Resolver::new(&notes);
    let broken = notes
        .iter()
        .flat_map(|n| n.links.iter())
        .filter(|l| resolver.resolve(l).is_none())
        .count();
    println!("{broken} broken links found: {:?}", t.elapsed());
}
