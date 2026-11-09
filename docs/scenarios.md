# Scenarios

Each scenario runs against the demo notebook. `crates/core/tests/scenarios.rs` checks all of them
on every build, and CI walks through them with the CLI on Windows, macOS and Linux.

Make the demo first. In the app use **Settings › Try a demo notebook** (it lands in
`~/FastMind Demo`); in a terminal run:

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

## 5. Dream with a model (optional)

In **Settings › Model** choose **Local server** (with Ollama running: `ollama run llama3.2`) or
**Claude** (with `ANTHROPIC_API_KEY` set). Dream now asks the model to title and summarize
each topic note, and Remember writes its answers in prose, citing notes as [1], [2]. If the
model can't be reached, both fall back to the offline way and say so.
