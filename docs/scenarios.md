# Scenarios

Each scenario runs against the demo notebook. `crates/core/tests/scenarios.rs` checks all of them
on every build, and CI walks through them with the CLI on Windows, macOS and Linux.

Make the demo first. On the first run choose **Try a demo notebook first**, or later use
**Settings › Try a demo notebook** (it lands in `~/FastMind Demo`). In a terminal run:

```sh
fastmind demo ~/FastMind-Demo
```

Making the demo switches FastMind to it. Switch back in Settings, or run
`fastmind --notebook <dir>` for one command. If you want to keep your real settings untouched,
set `FASTMIND_CONFIG` to a scratch file first.

## 1. Capture from anywhere, find it in a second

1. Press **Ctrl+Alt+Space** (**Cmd+Option+Space** on macOS), or tray › Capture…
2. Type `Flaky test in checkout: the payment mock times out after 2s on CI. #ci` and on a new
   line `Raise the mock timeout to 10s.`, then press **Ctrl+Enter**. The box closes.
3. Press **Ctrl+Alt+R** and type `payment mock`. The note is the first hit. `#ci` lists only
   notes tagged `ci`.
4. Type `what timeout should the payment mock use?` and press **Enter**. The answer quotes
   "Raise the mock timeout to 10s." with **[1]** linking to the note.

On the CLI: `fastmind capture "…"`, `fastmind search "payment mock"`, `fastmind ask "…?"`.

## 2. Dream compresses a messy notebook

1. Open **Dream** and press **Dream now**. Or wait for the night: Dream runs once between 02:00
   and 06:00 while FastMind is running.
2. The summary reads: 22 notes, **22 → 10**, about 48 KB lighter. The proposals are:
   - Merge "Faster docker builds" and "docker layer caching" into "Docker build cache"
   - Merge "Redis eviction" into "Redis eviction policy"
   - Gather 4 short notes into "Kubernetes probe"
   - Gather 3 short notes into "Sourdough starter"
   - Archive "Meeting room booking" (300 days untouched), "Conference wifi" (400 days),
     "Migrate CI to GitHub Actions" (done 75 days ago), "Untitled" (empty) and the image
     "old architecture.png" that nothing links to
   - Point `[[Postgress vacuum]]` in "Payments service" at `[[Postgres vacuum]]`
   - Pack the 2 notes archived two months ago. This one is already applied: packing runs
     without asking by default.
3. Press **Preview** on the Kubernetes topic to see the note it will write.
4. Press **Accept all**. The notebook now has 10 notes and every link still resolves. "Redis
   eviction policy" kept the line only the copy had ("Watch evicted_keys…") and answers to the
   copy's title as an alias.

Never proposed: "Work" (a `type: project` home note) and "Reading list" (`pin: true`).

## 3. Undo any part of it

1. On the Kubernetes topic card press **Undo**. The four fragments are back where they were, the
   topic note is gone, and everything else Dream did stays done.
2. Or open **History**: every change is listed with **Undo**.
3. Edit "Redis eviction policy" in your editor and save, then try to undo its merge. FastMind
   refuses ("changed since FastMind wrote it") and your edit stays.

## 4. Get an archived note back

1. Search for `conference wifi`. The archived note still shows up, marked *archived* and ranked
   below live notes.
2. Open **Archive**: "Q1 planning" and "Old blog draft" are *packed* into a zip.
3. Press **Restore** on "Q1 planning". It's back at `work/Q1 planning.md`, unzipped.

## 5. FastMind needs its AI

1. On a fresh install the welcome screen starts with **Your AI**. Claude Code is chosen when it's
   installed; the Claude API is offered when `ANTHROPIC_API_KEY` is set; a local server always.
2. **Start** checks the AI answers ("✓ Claude Code answered in 4.0 s") before anything is saved.
   If it doesn't, it says why, and nothing changes.
3. A notebook set up before, with no AI chosen, shows only the AI step.
4. From a terminal: `fastmind search wifi` says "FastMind needs an AI provider…";
   `fastmind model claude-code` checks it and saves it.
5. In Dream, every proposal's reason is the AI's sentence about those notes, and merged and
   topic notes are written by it. Edit one of a merge's notes before accepting: the merge
   becomes *out of date* rather than overwriting your edit.

## 5b. Captures written into finished notes

1. Optionally, in **Settings › Model**, say how you like notes written ("Always in English.
   Short.").
2. Capture something rough: `standup: Mara says the EU invoices batch moved to 02:00 UTC
   because of the bank's new cutoff. need to update the runbook and tell support`.
3. The toast says "Saved as a fresh memory. Writing it up…". In **Fresh memories** the card shows
   "Writing the note from what you captured…" and, a few seconds later, becomes **EU invoices
   batch moved to 02:00 UTC ✦** with a summary, key points, two tasks, a link to
   `[[Payments service]]` and "File in ❖ Work" with the model's reason.
4. Capture a link with a comment, or a screenshot of an error with "after the deploy". The
   model reads the page (redirect pages are followed) or the image, and says so in the note
   when something couldn't be read rather than inventing it.
5. **Read** the note: **Original** shows what you captured, as you gave it. **Write again**
   writes it afresh (even after you edited it); **History** undoes that.
6. Edit a captured note before the model is done: your edit wins, and the note is left as you
   wrote it.

## 6. Settle a fresh memory now, if you want

1. Open **Fresh memories**. Each note shows where it probably belongs, from the notes it's most like:
   "File in ▸ ideas/", with "Beside Docker build cache" as the reason. A topic note is offered
   with ❖.
2. Press **Enter** to file the first note there. The next note is selected. **F** files in
   another folder (new folders are made), **K** keeps a note where it is so Dream won't move it, **A** archives it. Left alone, each
   one settles overnight.
3. Filing into a topic note moves the note beside it and adds `- [[the note]]` to its
   `## Notes` list. **History** can undo either.

## 7. Turn AI sessions into memory

1. **Settings › Learn from AI sessions**: tick **Turn AI sessions into notes**, then press
   **Import now**. FastMind reads Claude Code's transcripts (`~/.claude/projects`) and Codex's
   (`~/.codex/sessions`) on this computer.
2. Each finished session becomes a note in `AI sessions/<project>/`. It holds what you asked,
   how it ended, the files changed and the commands run. API keys, tokens, private keys and
   passwords are replaced by `<redacted>`. Subagents' side conversations, scripted runs and
   sessions in temporary folders are left out.
3. Search for something you solved with an agent last week: the session comes up, marked
   **AI session**.
4. Overnight, Dream gathers sessions that continued the same work ("Multi notebook tabs",
   "Multi notebook tabs 2") into one thread note, and archives sessions nobody opened in 90 days.

## 8. Give your coding agent the notebook

1. **Settings › Connect your coding agents** shows the command. For Claude Code:
   `claude mcp add --scope user fastmind -- <path>/fastmind mcp`.
2. In a Claude Code session ask "search my notes for the ledger lag". It calls `search_notes`
   and reads the note.
3. When the agent finds something worth keeping, it calls `save_finding`. The finding lands in
   your fresh memories tagged `agent`, with secrets removed.

## 9. The first run

On a computer where FastMind hasn't been set up, the app asks where your notes are. It offers:

- a notebook FastMind used before
- FastCapture's notebooks and Obsidian vaults it finds
- a new `~/FastMind`, any folder you type or browse to, or the demo

It also shows the two hotkeys and, if it finds Claude Code or Codex sessions, offers to turn them
into notes. Nothing is written until you choose.
