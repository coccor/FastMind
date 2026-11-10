# FastMind: vision and design

## Vision

A developer never loses a thought, a finding or a fix. Anything worth keeping is saved in a
second, comes back when it matters, and stays in plain files that you, your editor and your
coding agents share. The notebook tidies itself overnight, and nothing changes without your say.

What we bet on:

1. People keep notes only when saving costs nothing: one hotkey, nothing to file.
2. People don't trust background changes to their notes unless each one is proposed first and can
   be undone.
3. Plain Markdown that works in any editor beats lock-in.
4. Notebooks grow until they're noise. Compressing them (merging, gathering, retiring) is what
   keeps search useful. Dream does that work.

What it is not: an editor (that's FastPad), cloud sync, a team wiki or a plugin platform.

## Decisions taken building 0.1

- **A fresh, cross-platform codebase.** FastCapture is written directly against Win32, about
  42k lines. FastMind is a Rust core with no OS code, plus a Tauri 2 shell (tray, global
  hotkeys, webview) that runs on Windows, macOS and Linux.
- **One command surface.** `fastmind_core::api::App::call(name, json)` serves the desktop app,
  the CLI and `fastmind serve` alike, so they can't drift apart. The UI is plain HTML/JS: in
  Tauri it calls `invoke("call")`, in a browser it posts to `/api/<name>`.
- **No database.** The search index is built in memory from the files and rebuilt every few
  seconds when needed. Everything FastMind keeps lives in `<notebook>/.fastmind/`:
  - `dream/latest.json`: the latest report and the status of each proposal
  - `dream/log.jsonl`: one line per night
  - `journal/<id>/`: one folder per change, holding a manifest and copies of the files as they
    were
  - `archive/files/…`: archived notes; `archive/items/<id>.json` describes each one;
    `archive/packs/*.zip` holds packed ones
  - `usage.json`: which notes you opened from FastMind, and when
- **Undo checks every file first.** Each journal entry records the hash each file had after the
  change. Undo restores the copies only if every file still has that hash. Each archived item
  has its own small file, so archive changes don't block each other's undo.
- **Offline first.** Similarity uses tf-idf cosine for shared vocabulary and 3-word shingle
  containment for shared phrasing. Topic titles come from the word most fragment titles share,
  plus the word or tag most of them use; summaries are the sentences closest to the group's
  key words. Answers quote the best-matching sentences. A model only improves titles,
  summaries and answers.
- **Claude through plain HTTP.** Rust has no official Anthropic SDK, so `model.rs` calls the
  Messages API directly: `claude-opus-5-5` at low effort, with server-side refusal fallback.
  The key comes from `ANTHROPIC_API_KEY`. "Local" speaks the OpenAI-compatible chat API that
  Ollama, LM Studio and llama.cpp offer.

## Dream, step by step

1. Read every note. Skip hidden folders, `.fastmind/` included.
2. Leave out protected notes (home types, pinned, `README`/`index`/`home`) and anything touched
   in the last two days.
3. **Duplicates:** compare candidate pairs that share one of their 12 strongest words. Two notes
   are duplicates when their vocabulary agrees (cosine ≥ 0.45) and 55% of the shorter note's
   phrasing is in the longer one. Groups form transitively, and the copy with the most inbound
   links stays.
4. **Topics:** take fragments (3–120 words) not already in a merge. Similarity is cosine, plus
   0.1 when two notes share a tag. Clusters grow around the note with the most neighbours; a
   newcomer must be close to at least half the group. A cluster needs 3 or more notes.
5. **Archive:** no inbound links and not used since the cutoff: 180 days idle, 30 days after
   `status: done`, or empty after a day. "Used" is the newest of the file time, the dates in
   the front matter and the last open from FastMind. Attachments count when no note mentions
   their file name.
6. **Links:** an unresolved link whose target is within one or two typos of exactly one note.
7. **Pack:** loose archive items and undo records older than their cutoffs.
8. Save the report, then apply the kinds allowed to run without asking.

Applying a proposal recomputes it from the current files, so the proposals are independent:
accepting one doesn't make another stale. If a file it needs is gone, the proposal is marked
*out of date*.

## Since the first version

- **AI sessions** (`sessions.rs`). Claude Code transcripts are JSONL. User and assistant
  lines carry `cwd`, `gitBranch`, `entrypoint` and `isSidechain`, and `ai-title` lines name the
  session. Subagent files (in `subagents/`), `sdk-*` entrypoints and sessions in temporary
  folders are skipped. Notes take the session's end time as their file time, so Dream judges
  them by when the work happened. A note edited by hand is never overwritten when the session
  grows.
- **Threads.** AI sessions in one project whose titles match after a trailing copy number is
  dropped ("Backlog creation 2"), or share most of their words (two or more), are gathered once
  the newest is three days quiet.
- **Records and templates.** Daily notes (named by the date alone, or in a daily or journal
  folder), meetings and logs are never archived for being old, merged or gathered. Templates
  are never touched.
- **Skips are remembered** by signature (the proposal's kind and its sorted notes). A new copy
  changes the signature, so Dream may ask again.
- **Filing** (`filing.rs`). Suggestions add up the similarity of a note's 20 nearest notes per
  folder, and weight home notes 1.5×. A Markdown link that points at a moved note is rewritten
  relative to the note holding it.
- **Speed.** Search keeps one cache of notes and documents, refreshed from the directory
  listing's sizes and times: about 13 ms for 5,000 notes when nothing changed, and only changed
  files are read. Dream compares only pairs that share one of their strongest words, and looks
  for link fixes among names with the same first letter.
- **Writing captures** (`write.rs`), FastCapture's two steps. Step one saves the note as typed
  and keeps the capture itself in `.fastmind/raw/<id>.json` (text, link, tags, stored files,
  the note's path and hash); the note carries `capture_id`. Step two, on a background thread,
  fetches each link's main text (article, else main, else body, without scripts and
  navigation; `meta refresh` followed), reads text attachments and up to five images, and
  gives the model the capture in delimited blocks: `<your_note>`, `<source>`,
  `<attachment>`, `<candidate_notes>` (the 10 best search hits plus the notebook's home
  notes), `<home_folders>` and the most used tags. The rules are FastCapture's; the user's own
  preferences follow them. The reply is JSON (structured output on the Claude API and Claude
  Code); a reply that can't be used goes back once with what was wrong. Links and homes the
  model made up are dropped, tags are cleaned and capped. The note is replaced in one journal
  transaction, only if its hash is still the one FastMind wrote, and renamed after its new
  title while it's in the inbox. Writing again is explicit and works on edited or filed notes
  (a filed note keeps its name and place). Captures saved while no model ran are written when
  the app next runs, if from the last week and untouched. **Claude Code** is run as
  `claude -p` with stream-json in and out, no tools, no MCP servers, no settings and no
  `ANTHROPIC_*` variables, in an empty folder.
- **MCP** (`crates/cli/src/mcp.rs`): newline-delimited JSON-RPC on stdio, protocol 2025-06-18,
  with four tools.

## Not done yet

- Installers (MSI/DMG/AppImage through `tauri build`) and signing; CI uploads raw binaries
- Remind, Focus and Practice from the backlog; Remember by meaning (embeddings)
- Idle detection for Dream (it runs in its time window; it doesn't wait for the machine to be idle)
- PDF and Word text isn't extracted for the writing step yet (FastCapture's extractors aren't
  ported); such attachments are named to the model as not read
- Kept captures (`.fastmind/raw/`) aren't packed or pruned yet
