# Licenses

## FastMind

FastMind source code and binaries: MIT License, Copyright (c) 2026 Cocioaba Cornel. The full
text is in `LICENSE`.

## Diagrams

FastMind draws `.fdraw` diagrams with `fastdraw-render`, built from the FastDraw source. It
needs no network and no other program. The icons and fonts below are embedded in the program.

| Component | Version | License | Text |
|---|---|---|---|
| FastDraw (`fastdraw-render`, linked) | 0.2.2, git `f0d3905` (not on crates.io) | MIT License, Copyright (c) 2026 coccor | `licenses/FastDraw.txt` |
| Phosphor Icons (diagram icons) | pinned with `fastdraw-render` | MIT License, Copyright (c) 2023 Phosphor Icons | `licenses/phosphor-icons.txt` |
| Geist and Geist Mono fonts | pinned with `fastdraw-render` | SIL Open Font License 1.1, Copyright 2026 The Geist Project Authors | `licenses/OFL-Geist.txt` |
| Instrument Serif font | pinned with `fastdraw-render` | SIL Open Font License 1.1, Copyright 2022 The Instrument Serif Project Authors | `licenses/OFL-InstrumentSerif.txt` |
| Libre Franklin font | pinned with `fastdraw-render` | SIL Open Font License 1.1, Copyright 2020 The Libre Franklin Project Authors | `licenses/OFL-LibreFranklin.txt` |
| Newsreader font | pinned with `fastdraw-render` | SIL Open Font License 1.1, Copyright 2020 The Newsreader Project Authors | `licenses/OFL-Newsreader.txt` |
| Sketch Hand font, a modified Virgil | pinned with `fastdraw-render` | SIL Open Font License 1.1, Copyright (c) 2021 - Present Ellinor Rapp | `licenses/OFL-SketchHand.txt` |

`fastdraw-render` brings three more crates: `indexmap` 2.14.2 (Apache-2.0 OR MIT), its
dependencies `equivalent` 1.0.3 (Apache-2.0 OR MIT) and `hashbrown` 0.17.1 (MIT OR Apache-2.0),
and `ryu-js` 1.0.3 (Apache-2.0 OR BSL-1.0, used here under Apache-2.0).

## Rust crates

Every other Rust crate is listed with its license by `cargo metadata --locked`, which reads
`Cargo.lock`.
