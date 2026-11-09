<div align="center">

<img src="apps/desktop/ui/icon.svg" alt="FastMind" width="88" height="88">

# FastMind

**Catch it now. Find it later. It tidies itself while you sleep.**

One small app in your tray, on Windows, macOS and Linux, over a folder of plain Markdown notes.

</div>

FastMind does three things a mind does:

- **Capture.** One hotkey from anywhere opens a small box. Type, press **Ctrl+Enter**, and the
  thought is a note in your inbox. No model, no waiting.
- **Remember.** Another hotkey searches the whole notebook as you type, typos and all. End with
  a `?` and it answers from your notes, citing each one it used.
- **Dream.** At night, Dream compresses the notebook. It merges notes that say the same thing,
  gathers scattered fragments into one topic note, archives what nothing uses any more, fixes
  links that point nowhere and packs old archives into zip files. It **proposes**; you accept
  what you like in the morning. **Every change can be undone.**

Your notes stay plain `.md` files with front matter, readable by any editor, Obsidian,
[FastPad](https://github.com/coccor/FastPad) or an AI agent. FastMind keeps its own state in a
`.fastmind/` folder inside the notebook: Dream's proposals, the undo journal and the archive. There's
no database to lose, and no account.

## Try it in two minutes

```sh
cargo run -p fastmind-desktop          # the app, in the tray
```

In the app open **Settings › Try a demo notebook**, then **Dream › Dream now**. The demo is a
messy notebook with 22 notes: copies, fragments, finished and forgotten notes, an empty note, a
mistyped link and an unused image. Dream proposes taking it to 10 notes. Preview any proposal,
accept it, then undo it from the card or from **History**.

Or use the terminal:

```sh
cargo run -p fastmind-cli -- demo ~/FastMind-Demo
cargo run -p fastmind-cli -- dream
cargo run -p fastmind-cli -- dream apply all
cargo run -p fastmind-cli -- ask "how do I stop kubernetes restart loops?"
```

[docs/scenarios.md](docs/scenarios.md) walks through every scenario, step by step.

## Dream: compression you can trust

| Proposal | When | What happens |
|---|---|---|
| **Merge** | Most of one note's phrasing appears in another (55% by default) | The best-linked copy stays. Paragraphs only the others had are added under it. Their titles become `aliases`, links to them are pointed at the survivor, and the copies go to the archive. |
| **Gather a topic** | Three or more short notes share their vocabulary (a shared tag counts in favour) | One `type: topic` note holds them all in date order, under a title and summary taken from the notes. The fragments go to the archive and links follow. |
| **Archive** | Untouched, unlinked and unopened for 180 days, marked `done` for 30 days, or empty. Also files nothing links to. | Moved to `.fastmind/archive/`. Still searchable, ranked lower, and restored with one click. |
| **Fix a link** | `[[Postgress vacuum]]` points nowhere and only one note is that close | The link is corrected. |
| **Pack** | Archived notes older than 30 days, and undo records older than 30 days | Compressed into one zip in `.fastmind/archive/packs/`. Archived notes can still be restored from it. |

The rules that make this safe:

- **It proposes; it doesn't rewrite.** Only the kinds you allow are applied on their own. That is
  packing, by default (Settings › Dream).
- **Every change goes through a journal.** The files are copied before they change. Undo puts
  them back, but it refuses if you've edited a file since, so it never loses your work.
- **Some notes are never touched:** home notes (`type: topic`, `project`, `moc`, `index`), pinned
  notes (`pin: true`) and anything changed in the last two days.
- **Nothing is deleted.** Archiving moves a note; packing compresses it. Both can be reversed.
- **It works offline.** Dream and Remember run on text statistics. A model is optional: a local
  server (Ollama, LM Studio) or Claude, if you choose it. With a model, topic titles,
  summaries and answers read better.

## Keys

| Key | |
|---|---|
| **Ctrl+Alt+Space** (anywhere) | Capture box. **Cmd+Option+Space** on macOS. |
| **Ctrl+Alt+R** (anywhere) | Remember: search and ask |
| **Ctrl+Enter** | Save a capture, or ask the question in the search box |
| **Ctrl+K** / **Ctrl+N** in the app | Search / capture |

Both global keys can be changed in Settings. The tray menu has Capture, Remember, Dream now,
Dream's proposals, Open notebook folder, Settings and Quit. Closing the window keeps FastMind in the tray.

## The command line

`fastmind` does everything the app does, for scripts and coding agents:

```
fastmind capture "lag spikes during ledger batch" --tag perf --attach ./ledger.log
echo "from a pipe" | fastmind capture -
fastmind search "#perf ledger"
fastmind ask "why did the ledger batch lag?"
fastmind dream | dream show | dream apply p3 | dream apply all --kinds archive,pack | dream undo p3
fastmind history | undo <entry> | archive | restore <id> | inbox | status
fastmind serve            # the app's UI in a browser at http://127.0.0.1:7878 (localhost only)
```

`--notebook DIR` uses another notebook for one command, and `--json` prints raw JSON.

## Build

You need Rust (stable). On Linux the desktop app also needs WebKitGTK and the tray libraries:

```sh
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
cargo test --workspace
cargo build --release -p fastmind-desktop -p fastmind-cli
```

CI builds and tests on Windows, macOS and Linux, and runs the scenario walk-through on each.

| Folder | |
|---|---|
| `crates/core` | Everything that matters: notebook, capture, search, answers, Dream, journal, archive. No UI and nothing OS-specific. |
| `crates/cli` | `fastmind`, the terminal front end and `serve` |
| `apps/desktop` | The tray app (Tauri 2): hotkeys, windows, the nightly run. Its UI lives in `apps/desktop/ui`. |

## Settings

Settings are kept in `settings.json` in your config folder (`%AppData%\coccor\FastMind\config`,
`~/Library/Application Support/dev.coccor.FastMind`, `~/.config/fastmind`).
`FASTMIND_CONFIG` points somewhere else. The notebook defaults to `~/FastMind`.

## License

MIT
