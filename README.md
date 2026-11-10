<div align="center">

<img src="apps/desktop/ui/icon.svg" alt="FastMind" width="88" height="88">

# FastMind

**Catch it now. Find it later. It tidies itself while you sleep.**

One small app in your tray, on Windows, macOS and Linux, over a folder of plain Markdown notes,
and the memory your AI coding sessions never had.

</div>

## What it does

- **Capture from anywhere.** One hotkey opens a small box over whatever you're doing. Type,
  paste a screenshot or drop a file, press **Ctrl+Enter**, and you're back. A link gets its
  page's title and description. Esc keeps a draft.
- **Captures become finished notes.** With a model chosen, each capture is written up a few
  seconds after you save it: a clear title, a one-sentence summary, your text tidied in your own
  voice, key points, tasks, links to the notes it relates to, and where it belongs. The model
  reads the pages you linked, the screenshots and the text files you attached, and the notes
  and folders you already have. What you captured is always kept, a note you've edited is never
  rewritten, and History undoes it. **Claude Code** works with your Claude plan, no API key.
- **File in one key.** The inbox suggests where each note belongs, from the notes it's most like
  (a folder, or a topic note that gets a link to it). **Enter** files it there.
- **Remember.** Another hotkey searches everything as you type, typos and all. End with `?` and
  it answers from your notes, citing each one it used. Read and edit notes in the app, with
  related notes and backlinks beside them.
- **Dream.** At night Dream compresses the notebook. It merges notes that say the same thing,
  gathers scattered fragments (and AI sessions about the same work) into one note, archives what
  nothing uses any more, fixes links that point nowhere, and zips old archives. It **proposes**,
  you accept in the morning, and **every change can be undone**.
- **Learn from AI sessions.** Your Claude Code and Codex sessions become notes: what you asked,
  how it ended, the files changed, the commands run. Secrets are removed first. Sessions about
  the same work are gathered into one thread overnight.
- **Memory for your coding agents.** `fastmind mcp` lets Claude Code, Codex or Claude Desktop
  search the notebook before they dig into something, and save what they find to your inbox.
- **Home, every morning:** what Dream did and proposes, the inbox, recent notes, and two older
  notes brought back so the good ones don't get lost.

Your notes stay plain `.md` files with front matter, readable by any editor, Obsidian,
[FastPad](https://github.com/coccor/FastPad) or an agent. FastMind keeps its own state in
`.fastmind/` inside the notebook (Dream's proposals, the undo journal, the archive). There's no
database to lose and no account, and nothing leaves your computer unless you choose a cloud model.

## Try it

```sh
cargo run -p fastmind-desktop
```

The first run asks where your notes are. It finds Obsidian vaults and FastCapture notebooks,
or makes `~/FastMind`. Choose **Try a demo notebook first** to see Dream work: a messy notebook
with 22 notes that Dream proposes taking down to 10. [docs/scenarios.md](docs/scenarios.md)
walks through every scenario.

## Dream: compression you can trust

| Proposal | When | What happens |
|---|---|---|
| **Merge** | Most of one note's phrasing appears in another | The best-linked copy stays, with what only the others said added under it. Their titles become aliases, links follow, and the copies go to the archive. |
| **Gather** | Three or more short notes share a subject, or AI sessions continue the same work | One note holds them in date order. The pieces go to the archive and links follow. |
| **Archive** | Untouched, unlinked and unopened for 180 days, `done` for 30, empty, or an AI session unopened for 90. Also files nothing links to. | Moved to `.fastmind/archive/`: still searchable, ranked lower, and restored in one click. |
| **Fix a link** | `[[Postgress vacuum]]` points nowhere and one note is that close | The link is corrected. |
| **Pack** | Archived notes and undo records over 30 days old | Zipped into `.fastmind/archive/packs/`. Notes can still be restored from the pack. |

What keeps it safe:

- It proposes; you decide. Only the kinds you allow run on their own (packing, by default).
  **Skip** means Dream won't propose that again, and **Keep forever** pins a note.
- Every change goes through a journal. Undo puts the files back, but it refuses if you've
  edited them since.
- Some notes are never touched: home notes (`type: topic`, `project`, `moc`), pinned notes,
  templates, records (daily notes, meetings, logs) and anything changed in the last two days.
- Nothing is deleted, and it works offline. A model (a local server, or Claude) only writes
  better titles, summaries and answers.

It's tested against real-world shapes: templated meeting and daily notes are never mistaken for
copies. On 5,000 notes Dream takes about 4 seconds and search about 20 ms.

## Keys

| Key | |
|---|---|
| **Ctrl+Alt+Space** (anywhere) | Capture. **Cmd+Option+Space** on macOS. |
| **Ctrl+Alt+R** (anywhere) | Remember: search and ask |
| **Ctrl+Enter** | Save a capture, or ask the question in the search box |
| **↑ ↓ / Enter** | Move through results / read |
| **Ctrl+E**, **Ctrl+S** | Edit the open note, save it |
| Inbox: **Enter**, **F**, **K**, **A** | File in the suggestion, another folder, keep, archive |
| **Ctrl+K** / **Ctrl+N** | Search / capture, inside the app |

Hotkeys can be changed in Settings and apply at once. Closing the window keeps FastMind in the
tray, where it dreams at night.

## Coding agents

Settings › **Connect your coding agents** shows the exact command for this computer. For
example, for Claude Code:

```sh
claude mcp add --scope user fastmind -- fastmind mcp
```

Agents get `search_notes`, `read_note`, `ask_notebook` and `save_finding`. Their findings land
in your inbox tagged `agent`, with secrets removed.

## The command line

`fastmind` does everything the app does:

```
fastmind setup ~/Notes                    # choose the notebook
fastmind capture "lag spikes in the ledger batch" --tag perf --attach ./ledger.log
fastmind write inbox/Lag.md               # write a captured note (again) with the model
fastmind search "#perf ledger"
fastmind ask "why did the ledger batch lag?"
fastmind dream | dream apply all | dream undo p3
fastmind history | undo <entry> | archive | restore <id> | inbox | status
fastmind mcp                              # the notebook for coding agents (MCP over stdio)
fastmind serve                            # the app's UI in a browser, on localhost only
```

## Build

You need Rust (stable). On Linux the desktop app also needs WebKitGTK and the tray libraries:

```sh
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
cargo test --workspace
cargo build --release -p fastmind-desktop -p fastmind-cli
```

| Folder | |
|---|---|
| `crates/core` | Notebook, capture, search, answers, Dream, filing, AI sessions, journal, archive. No UI and nothing OS-specific. |
| `crates/cli` | `fastmind`: the terminal, `serve` and `mcp` |
| `apps/desktop` | The tray app (Tauri 2): hotkeys, windows, nightly runs. Its UI is in `apps/desktop/ui`. |

Settings live in `settings.json` in your config folder (`%AppData%\coccor\FastMind\config`,
`~/Library/Application Support/dev.coccor.FastMind`, `~/.config/fastmind`). `FASTMIND_CONFIG`
points elsewhere.

## License

MIT
