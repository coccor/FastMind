# FastDraw Diagrams Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** FastMind shows `.fdraw` diagrams in notes, and Dream proposes diagrams where they help someone understand a note, both inside merged/topic notes and for existing notes.

**Architecture:** A new `diagram` module wraps `fastdraw-render` (look-path rule, embedded fonts, errors as `Line N: …`). The `image` API serves diagrams as SVG data URLs. Dream's merge/topic answers gain optional diagram fields; a new `Kind::Diagram` asks about a few existing notes at the end of the night, from a reserve of 3 requests. Broken diagrams get one fix request in that same final pass.

**Tech Stack:** Rust 2024 (`fastmind-core`), `fastdraw-render` (git, rev `f0d3905`), `base64`, plain JS UI (`apps/desktop/ui/app.js`).

**Spec:** `docs/superpowers/specs/2026-11-17-fastdraw-diagrams-design.md`

## Global Constraints

- Dependency: `fastdraw-render = { git = "https://github.com/coccor/fastdraw", rev = "f0d3905" }` in `[workspace.dependencies]`, used by `crates/core`.
- Look-path refusal text: `FastMind reads look files only by a relative path; "<path>" was not read`.
- Panic text: `FastDraw could not draw this diagram`. Size limit text: `diagram too large` (text over 1,000,000 bytes).
- Error strings are `Line <n>: <message>`.
- Diagram file: `<note folder>/attachments/<note stem>.fdraw`, free name at apply time; link `![<caption>](attachments/<name with spaces as %20>.fdraw)`.
- Reserve: `min(3, model_requests)` requests, only when `diagram_notes > 0`. `DreamSettings::diagram_notes` default `3`.
- Candidates: body ≥ 250 words, no image/diagram link, not protected/record/template/AI session/thread/inbox, untouched for 2 days, not in another proposal this night.
- `Kind::Diagram` serialises as `"diagram"`; `Kind::parse` accepts `"diagram"` and `"draw"`. Not in default `auto_apply`.
- Old `latest.json` without the new fields must still load (`#[serde(default)]`).
- Every model-written diagram that reaches a proposal has zero `Error` issues. Warnings are ignored.
- Project style: plain prose comments, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.

## Review Focus

1. A diagram whose note moves (Settle/filing) keeps a working link: the `.fdraw` sits in `attachments/` beside the note, the same as captured images. Task 3 adds a test that settling a note with a diagram keeps the image resolvable.
2. A model answer whose `diagram` is whitespace or a fenced block (```` ```fdraw … ``` ````) is treated as "none" or unfenced, not drawn as garbage. Task 4 adds `diagram_text_is_unfenced_and_blank_is_none`.
3. Two proposals for notes with the same stem in one folder must not write over each other's `.fdraw`. The name is chosen at apply time with `fsx::free_name`. Task 3 adds `second_diagram_gets_a_free_name`.
4. A note edited after Dream proposed its diagram is out of date, not rewritten. Task 5 adds `edited_note_makes_diagram_outdated`.
5. Dark mode: a diagram drawn with `dark: true` uses the look's dark paper. Task 1 adds `dark_draw_differs`.

---

### Task 1: The `diagram` module

**Files:**
- Modify: `Cargo.toml` (workspace dependency), `crates/core/Cargo.toml`, `crates/core/src/lib.rs` (`pub mod diagram;`)
- Create: `crates/core/src/diagram.rs` (unit tests inside, as other modules do)

**Interfaces:**
- Produces:
  - `pub struct Drawn { pub svg: String, pub errors: Vec<String> }`
  - `pub fn draw(text: &str, file: Option<&std::path::Path>, dark: bool) -> Drawn`
  - `pub fn errors(text: &str) -> Vec<String>` (same as `draw(text, None, false).errors`)
  - `pub fn unfence(text: &str) -> Option<String>` (trims; strips one surrounding ```` ``` ````/```` ```fdraw ```` fence; `None` when empty)
  - `pub const MAX_BYTES: usize = 1_000_000;`

- [ ] **Step 1: Write failing tests** in `diagram.rs`:
  - `draws_a_valid_diagram`: `draw("a: focal \"A\" at 40,40\nb: store \"B\" at 300,40\na -> b \"x\"", None, false)` has `errors.is_empty()`, `svg.starts_with("<svg")`, and `svg.contains("@font-face")`.
  - `unknown_shape_is_a_line_error`: line 2 uses an unknown style. `errors[0].starts_with("Line 2: ")`.
  - `refuses_absolute_look_paths`: for each of `//server/x.look`, `\\\\server\\x.look`, `C:/x.look`, `C:x.look` and `/x.look` as `look "<p>"` on line 1, plus one box, with `file: Some(tmp/d.fdraw)`: `errors == [format!("Line 1: FastMind reads look files only by a relative path; \"{p}\" was not read")]` and `svg` is non-empty.
  - `relative_and_bundled_looks_pass`: `look "night"` gives no errors. `look "x.look"` with `x.look` written beside the file in a tempdir gives no refusal error.
  - `too_large_is_refused`: text of `MAX_BYTES + 1` bytes gives `errors == ["diagram too large"]`.
  - `dark_draw_differs`: the same text with `dark: true` and `dark: false` gives different SVGs.
  - `diagram_text_is_unfenced_and_blank_is_none`: `unfence("  ")` is `None`. `unfence("```fdraw\na: focal \"A\" at 0,0\n```")` is `Some("a: focal \"A\" at 0,0")`.
- [ ] **Step 2: Run** `cargo test -p fastmind-core diagram::` and check that it fails to compile (no module).
- [ ] **Step 3: Implement.**
  - Add the dependency.
  - The look rule: refuse when `look_path` starts with `/`, `\`, or has `X:` at char 1. Then render with `file: None` and replace the issue on that line whose message starts with `look file not found` with the refusal text.
  - Wrap `render` in `std::panic::catch_unwind(AssertUnwindSafe(..))`.
  - Fonts: collect the `font-family` names used in the SVG. For each bundled font in `fastdraw_render::fonts()` whose family matches, and each file in `font_files`, insert one `<style>@font-face{font-family:"…";src:url(data:font/ttf;base64,…)}</style>` right after the opening `<svg …>` tag. To map a file name to its family, copy FastPad's `src/preview/fastdraw.rs` if it has the mapping, or read the TTF name table. Cache the base64 strings in a `OnceLock`.
  - Format errors as `Line {line}: {message}` and keep only `Level::Error`.
- [ ] **Step 4: Run** `cargo test -p fastmind-core diagram::`. Expect PASS.
- [ ] **Step 5: Commit** `feat(diagram): draw .fdraw text with fastdraw-render`

### Task 2: Show diagrams in notes and previews

**Files:**
- Modify: `crates/core/src/api.rs` (`"image"`, new `"diagram_draw"`), `crates/core/src/dream/mod.rs:234` (`ATTACHMENT_EXT` gains `"fdraw"`, array size 17), `apps/desktop/ui/app.js` (`renderMarkdown`, `loadImages`, Dream preview), `apps/desktop/ui/style.css` if needed
- Test: the `api.rs` test module (or `crates/core/tests/` next to existing API tests; follow wherever `"image"` is tested now)

**Interfaces:**
- Consumes: `diagram::draw`, `diagram::MAX_BYTES`
- Produces:
  - API `image {path, dark?: bool}`: for `.fdraw`, returns `{"data": "data:image/svg+xml;base64,…"}`. With errors it returns `Err(Error::msg(<errors joined "; ">))`.
  - API `diagram_draw {text, dark?: bool}` → `{"data": …}`, or the same error.
  - JS `loadImages(root, notePath, diagrams = {})`, where `diagrams` maps a `data-src` to `.fdraw` text drawn through `diagram_draw`.

- [ ] **Step 1: Failing tests:**
  - `image_draws_fdraw`: write `notes/attachments/d.fdraw` (valid). `call("image", {"path": "notes/attachments/d.fdraw"})` returns data starting with `data:image/svg+xml;base64,`.
  - `image_reports_diagram_errors`: a broken file gives an error containing `Line 1:`.
  - `diagram_draw_draws_text`.
  - In `dream_ai.rs`: `unlinked_fdraw_is_archived_like_an_image`. An unlinked `x/attachments/old.fdraw` 200 days old gets an archive proposal, as an unlinked `.png` does in the existing test.
- [ ] **Step 2: Run** `cargo test -p fastmind-core image_ diagram_draw unlinked_fdraw`. Expect FAIL.
- [ ] **Step 3: Implement.**
  - API: read at most `MAX_BYTES`, then `diagram::draw(&text, Some(&file), dark)`.
  - `app.js`: the wiki-embed regex also accepts `fdraw`. `loadImages` sends `dark: matchMedia("(prefers-color-scheme: dark)").matches`. On failure for a `.fdraw`, show `[diagram: <error>]` instead of `[missing image: …]`.
  - Preview handler: after `renderMarkdown(proposal.preview)`, call `loadImages(pane, previewNotePath(proposal), diagramsOf(proposal))`. `diagramsOf` maps `plan.diagram.file` to `plan.diagram.text`, and is empty when there is no diagram. `previewNotePath` is `plan.note`, `plan.keep` or `plan.path`. Fall back to the normal `image` call for other images.
- [ ] **Step 4: Run** the tests. Expect PASS. Run `cargo run -p fastmind-desktop`, open a note linking a `.fdraw`, and check that it draws, in light and dark.
- [ ] **Step 5: Commit** `feat: show .fdraw diagrams in notes`

### Task 3: Diagrams in plans: data, link insertion, apply and undo

**Files:**
- Modify: `crates/core/src/dream/mod.rs` (`Kind`, `Plan`, `execute`), `crates/core/src/dream/compose.rs` (link insertion)
- Test: `crates/core/tests/dream_ai.rs`, plus unit tests in `compose.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)] pub struct Diagram { pub file: String, pub caption: String, pub text: String }` in `dream/mod.rs`
  - `Plan::Merge { …, #[serde(default)] diagram: Option<Diagram> }` and `Plan::Topic { …, #[serde(default)] diagram: Option<Diagram> }`
  - `Plan::Diagram { note: String, diagram: Diagram }`, `Kind::Diagram`
  - `pub fn compose::with_diagram(note_text: &str, caption: &str, link: &str) -> String`: inserts `\n![caption](link)\n` after the first `# ` line, and after a blockquote that directly follows it. With no `# ` line, it goes at the top of the body, after the front matter.
  - `pub fn diagram_link(file_name: &str) -> String` → `attachments/<name, spaces as %20>`
  - `fn write_diagram(tx: &mut Transaction, nb: &Notebook, note_path: &str, note_text: &str, d: &Diagram) -> Result<String>`: picks the free name under `<folder>/attachments/`, writes the `.fdraw` and returns the note text with the final link.

- [ ] **Step 1: Failing tests:**
  - `compose::with_diagram_after_title_and_summary`: `"# T\n\n> s\n\nbody"` → `"# T\n\n> s\n\n![c](attachments/T.fdraw)\n\nbody"`.
  - `with_diagram_no_heading`: front matter is kept and the link goes first in the body.
  - `kind_parses_diagram`: `Kind::parse("draw") == Some(Kind::Diagram)`.
  - `old_report_loads`: a `latest.json` fixture with a topic plan and no `diagram` key deserialises.
  - `applying_diagram_plan_writes_file_and_link`: build a `Proposal` with `Plan::Diagram` by hand, save it as the latest report through the public API, and `dream::apply`. Then `notes/attachments/Note.fdraw` exists and the note contains `![c](attachments/Note.fdraw)`. `dream::undo` removes both.
  - `second_diagram_gets_a_free_name`: with `attachments/Note.fdraw` already there, applying writes `attachments/Note 2.fdraw` (whatever `fsx::free_name` yields) and links it with `%20`.
  - `topic_with_diagram_writes_both`: a `Plan::Topic` with `diagram` writes the topic note with the link and the `.fdraw`, in one journal entry.
  - `settled_note_keeps_its_diagram`: an inbox note with `![d](attachments/x.fdraw)` is settled. Its diagram still resolves, either by the same rule `loadImages` uses or because filing moves the attachment. Mirror whatever the existing image test checks.
- [ ] **Step 2: Run** `cargo test -p fastmind-core --test dream_ai` and the `compose` tests. Expect FAIL.
- [ ] **Step 3: Implement.**
  - In `execute`, `Plan::Diagram` checks `sources` (the existing loop already does), then calls `write_diagram` and `tx.write(note, …)`.
  - Merge and Topic call `write_diagram` on their text before writing it.
  - Update every `match` on `Plan` and `Kind` (`summarize`, `signature`, the CLI).
- [ ] **Step 4: Run** `cargo test --workspace`. Expect PASS.
- [ ] **Step 5: Commit** `feat(dream): diagram plans, link insertion, apply and undo`

### Task 4: Skills and diagrams in merge/topic answers

**Files:**
- Create: `crates/core/skills/fastdraw-language.md`, `crates/core/skills/draw-diagram.md`
- Modify: `crates/core/src/skills.rs` (`DEFAULTS`, `system_with`), `crates/core/skills/merge.md`, `gather-topic.md`, `gather-thread.md`, `crates/core/src/dream/ai.rs` (schemas, `Merge`, `Topic`), `crates/core/src/dream/mod.rs` (building merge/topic proposals and previews)
- Test: `crates/core/tests/dream_ai.rs`, the `skills.rs` unit tests

**Interfaces:**
- Consumes: `diagram::errors`, `diagram::unfence`, `compose::with_diagram`, `Diagram` (Task 3)
- Produces:
  - `pub fn skills::system_with(nb: Option<&Notebook>, name: &str, extra: &[&str], preferences: &str, contract: &str) -> String`. It appends each extra skill's body under its own heading before the guard. `system` calls it with `&[]`.
  - `Judge::prompt_with(&self, name: &str, extra: &[&str]) -> String`
  - `pub struct Drafted { pub caption: String, pub text: String, pub errors: Vec<String> }` in `ai.rs`
  - `ai::Merge` and `ai::Topic` gain `pub diagram: Option<Drafted>`
  - `pub fn ai::drafted(text: &Value, caption: &Value) -> Option<Drafted>`: unfences, then sets `errors = diagram::errors(..)`
  - Merge item schema and topic schema gain required string fields `diagram` and `diagram_caption` (`""` for none)

- [ ] **Step 1: Failing tests:**
  - `every_default_has_a_name_and_a_description` covers the two new skills.
  - `language_skill_follows_diagram_jobs`: `system_with(None, "merge", &["fastdraw-language"], "", JSON)` contains the language skill's top heading.
  - In `dream_ai.rs`, extend the fake `model`:
    - The topic answer includes a valid `diagram` and `diagram_caption: "How the starter is kept"`.
    - `topic_proposal_carries_its_diagram`: the topic plan's `diagram.file == "attachments/Sourdough starter.fdraw"`, and `preview` contains `![How the starter is kept](attachments/Sourdough%20starter.fdraw)` after the summary line.
    - `merge_without_diagram_has_none`: the merge answer has `diagram: ""`, so `diagram` is `None`.
    - `broken_topic_diagram_is_held`: a broken diagram leaves the proposal with `diagram` holding `Drafted.errors` non-empty internally. The final shape is checked in Task 5. Here, check that `run` produces the topic proposal at all (not dropped).
- [ ] **Step 2: Run.** Expect FAIL.
- [ ] **Step 3: Write the skills and wire them.**
  - `fastdraw-language.md` has front matter `name: fastdraw-language` and `description: FastDraw's diagram language, for the jobs that draw.` The body is condensed from `C:\Projects\fastdraw\guide\guide.md` (language sections only). Leave out commands, files and the workflow.
    - Cover elements and the built-in look's styles with default sizes. Run `npx fastdraw sizes` once while writing it and copy the numbers.
    - Cover `at x,y`, `w=`/`h=`, arrows and `style=`, labels, groups/containers, legend, and spacing and alignment rules.
    - Give one small worked example each for architecture, flowchart, process, state, layers, timeline, tree and sequence. Check every example with `npx fastdraw check` (no errors or warnings) before committing.
    - Rules: no `look` line; at most about 12 boxes; labels in the note's words; leave room, since nobody will see the picture before the user does.
  - `draw-diagram.md` gives the judgment from spec §7.4, the answer fields, and an example of `worth: true` and one of `worth: false`. Add a short "Diagram (optional)" section to merge, gather-topic and gather-thread. Every example JSON in those skills gains `"diagram": "", "diagram_caption": ""`.
  - Building proposals: when `Drafted.errors` is empty, set `plan.diagram` and build `preview` with `compose::with_diagram(text, caption, &diagram_link(<stem>.fdraw))`. When it isn't empty, keep the `Drafted` in a held list `Vec<(proposal_index, Drafted)>` for Task 5's pass. The plan has no diagram until fixed.
- [ ] **Step 4: Run** `cargo test --workspace`. Expect PASS.
- [ ] **Step 5: Commit** `feat(dream): merge and topic notes may come with a diagram`

### Task 5: Reserve and the final diagram pass (fixes and the diagram kind)

**Files:**
- Modify: `crates/core/src/config.rs` (`DreamSettings::diagram_notes: usize`, default `3`), `crates/core/src/dream/ai.rs` (`Judge` reserve, `draw`, `fix`), `crates/core/src/dream/mod.rs` (`run_in`: reserve at start, pass after settle; candidate selection in a new `fn diagram_candidates`)
- Test: `crates/core/tests/dream_ai.rs`

**Interfaces:**
- Consumes: Task 4's `Drafted` and held list; Task 3's `Plan::Diagram`, `compose::with_diagram`
- Produces:
  - `Judge::hold(&self, n: usize)` takes up to `n` from `budget` into a private `reserve: AtomicUsize`. `Judge::release(&self)` adds `reserve` back to `budget`.
  - `pub struct Drawing { pub worth: bool, pub caption: String, pub reason: String, pub diagram: Option<Drafted> }`
  - `pub fn ai::draw(judge: &Judge, note: &Note, used: &Mutex<HashSet<String>>) -> Option<Drawing>`: key `key("diagram", &[note])`, skill `draw-diagram` + `fastdraw-language`, note block up to 6000 chars. Schema `{worth: bool, kind, caption, diagram, reason}`, all required.
  - `pub fn ai::fix(judge: &Judge, skill: &str, key: &str, drafted: &Drafted, used: …) -> Option<Drafted>`: cache key `format!("{key}~fix")`. The user turn holds the diagram in `<diagram>` and the errors in `<errors>`, with "Fix only these errors; change nothing else." Schema `{diagram}`.
  - `fn diagram_candidates(notes: &[Note], taken: &HashSet<String>, settings: &DreamSettings, places: &Places, now) -> Vec<usize>`: filters per Global Constraints, ranks by structure score (builder's weights), truncates to `diagram_notes`.

- [ ] **Step 1: Failing tests** (fake model extended for `# Draw a diagram`, the top heading of `draw-diagram.md`):
  - `long_structural_note_gets_a_diagram_proposal`: a 300-word note about a request passing through services → one `Kind::Diagram` proposal. Its `preview` contains the link, and `sources` holds the note's hash.
  - `short_list_or_imaged_notes_are_not_asked`: a 100-word note, a 300-word note that is only a bullet list of facts, and a 300-word structural note with `![x](a.png)` cause no `# Draw a diagram` call (count calls in the fake).
  - `not_worth_it_is_remembered`: `worth:false` gives no proposal. A second `run` makes no new diagram call (cached).
  - `diagram_notes_zero_asks_nothing_and_reserves_nothing`: `diagram_notes = 0` and `model_requests = 3` with three merge groups → all three merges are asked.
  - `reserve_keeps_three_for_diagrams`: `model_requests = 4` with more merge candidates than that → merges get 1 request, and the diagram note is still asked.
  - `broken_diagram_fixed_in_final_pass`: the topic diagram is broken and the fix answer is valid → the topic plan has the diagram, the preview has the link, and exactly one fix call was made.
  - `broken_diagram_still_broken_is_dropped`: the fix answer is broken too → the topic plan has `diagram: None`, the proposal stays, and `model_notes` contains a line starting `1 diagram dropped: Line`.
  - `edited_note_makes_diagram_outdated`: after `run`, edit the note, then `dream::apply` gives `Status::Outdated`.
- [ ] **Step 2: Run.** Expect FAIL.
- [ ] **Step 3: Implement.**
  - In `run_in`, right after `Judge::new`: `if settings.diagram_notes > 0 { judge.hold(3) }`.
  - After the settle step and before Pack: `judge.release()`. Then run fixes for the held list (in order), then `ai::in_parallel` over the candidates with `ai::draw`. A `Drawing` whose diagram has errors gets one `fix`.
  - Exclude notes that are already in a proposal this night.
  - Proposal fields per spec §8.3: title `Diagram for "<title>"`; `bytes_freed = -(diagram bytes + link line bytes)`.
- [ ] **Step 4: Run** `cargo test --workspace`. Expect PASS.
- [ ] **Step 5: Commit** `feat(dream): diagram proposals for existing notes, with a reserved budget`

### Task 6: UI kind, docs and licences

**Files:**
- Modify: `apps/desktop/ui/app.js:651-652` (`KIND_ICON.diagram = "◇"`, `KIND_NAME.diagram = "Draw a diagram"`; the meta line shows `+ diagram` when `p.plan.diagram`), `docs/features.md` (Dream section: diagrams; new "Diagrams in notes" under "Your notes, your files"), `docs/reference.md` (`dream.diagram_notes`, kind `diagram`), `docs/design.md` (dependency bullet; Dream step "Diagrams" with the reserve), `README.md` (Dream feature cell mentions diagrams)
- Create: `LICENSES.md` and `licenses/` with FastDraw (MIT), Phosphor Icons (MIT) and the OFL fonts. Copy the texts from `C:\Projects\fastdraw\licenses` and `C:\Projects\fastdraw\fonts`, as FastPad's `LICENSES.md` does.

- [ ] **Step 1:** Make the UI changes. Run `cargo run -p fastmind-desktop`, choose the demo notebook, and Dream now with a model. Check that a diagram card shows, Preview draws the diagram, Accept writes it, and Undo removes it.
- [ ] **Step 2:** Write the docs and licences.
- [ ] **Step 3: Run** `cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo test --workspace`. Expect all clean.
- [ ] **Step 4: Commit** `docs: FastDraw diagrams; licences for the renderer's icons and fonts`
