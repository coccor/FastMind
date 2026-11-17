# FastDraw diagrams in FastMind

**Branch:** `feat/fastdraw-diagrams`. One PR.
**Upstream:** [coccor/fastdraw](https://github.com/coccor/fastdraw), Rust crate `fastdraw-render`.
**Prior art:** FastPad's `docs/superpowers/specs/2026-11-16-fastdraw-diagrams-design.md`; FastMind follows
its choices where they apply (pinned crate, look-path rule, image links).

## 1. Intent

Notes explain things: how a system fits together, a process, a decision and its options. Some of those
are understood faster as a picture. While dreaming, the AI should ask itself whether a diagram would
help someone understand the note, and when it would, draw one in FastDraw's `.fdraw` language and put
it in the note. FastMind must therefore also show `.fdraw` diagrams wherever it shows a note.

Success: a few times a week Dream proposes a diagram that makes a note clearer, and the user accepts
it. Anti-goals: diagrams on notes that don't need one (a diagram of three bullet points), broken or
unreadable diagrams reaching the user, diagrams eating the night's model budget so merges and topics
go unasked.

## 2. Goals and non-goals

Goals:

1. Notes show diagrams linked as images: `![caption](attachments/Name.fdraw)` draws in the note view
   and in Dream's previews, offline.
2. **Diagrams in written notes.** When Dream writes a merged note or a topic (or thread) note, the
   model may add one diagram in the same answer. It shows in the proposal's preview and is accepted
   with the note.
3. **Diagram proposals for existing notes.** A new Dream kind, `diagram`, proposes a diagram for a
   note that is already in the notebook. Accepting it writes the `.fdraw` file and links it from the
   note.
4. Every diagram Dream proposes has been drawn by `fastdraw-render` without errors.

Non-goals (this version): editing diagrams in FastMind; an "Open in FastDraw" button; fenced
```` ```fdraw ```` blocks; `fastdraw check`'s layout warnings (overlaps, crossing arrows), which the Rust
crate doesn't have; showing the model a picture of its diagram; diagrams in captures written by
`write.rs`; custom looks written by Dream.

## 3. Approaches considered

| Approach | Trade-off | Chosen |
|---|---|---|
| A. Optional diagram inside merge/topic answers only | No extra requests; only new notes get diagrams | Part of it |
| B. New `diagram` kind for existing notes | Reaches the notes that most need it; costs requests and needs candidate picking | Part of it |
| C. Merge/topic only flag "worth a diagram"; a later diagram job draws it | Keeps merge/topic prompts lean, but a diagram for a note that isn't applied yet, and two requests per note | No |

The user chose A and B together ("both, in one go").

## 4. Dependency

```toml
fastdraw-render = { git = "https://github.com/coccor/fastdraw", rev = "f0d3905" }
```

In `[workspace.dependencies]`, used by `crates/core`. Same commit as FastPad (crate 0.2.2). The API
used: `render(&Source) -> Drawing` (never fails; problems are `issues`), `look_path`, `fonts`,
`Level`, `VERSION`. Its README lists it.

## 5. Drawing a diagram (`crates/core/src/diagram.rs`, new)

```rust
pub struct Drawn { pub svg: String, pub errors: Vec<String> }   // errors as "Line 4: …"
pub fn draw(text: &str, file: Option<&Path>, dark: bool) -> Drawn;
pub fn errors(text: &str) -> Vec<String>;                        // draw(text, None, false).errors
```

- **Look-path rule** (FastPad §3.3): `look_path(text)` that is absolute, starts with `\\` or `//`, or
  has a drive prefix is refused: render with `file: None` and replace the crate's "look file not
  found" issue on that line with `FastMind reads look files only by a relative path; "<path>" was not
  read`. Relative paths and bundled look names pass, with `file` set to the diagram's path.
- `render` runs inside `catch_unwind`; a panic is the error "FastDraw could not draw this diagram".
- **Fonts.** The SVG names font families with no `@font-face`, and it is shown as an `<img>` data URL,
  which can't load page fonts. `draw` adds a `<style>` with `@font-face` rules (base64 TTF from
  `fastdraw_render::fonts()` and the drawing's `font_files`) for only the families the SVG uses.
  Builder may cache the encoded fonts.
- Text over 1 MB is not drawn ("diagram too large").

## 6. Showing diagrams

- The `image` API command (`api.rs`) accepts `.fdraw`: reads the file, `diagram::draw(text,
  Some(path), dark)`, returns `{"data": "data:image/svg+xml;base64,…"}`. It takes an optional `dark`
  argument; the UI passes `matchMedia("(prefers-color-scheme: dark)").matches`. A diagram with errors
  is an error (`Line 4: …`), and the UI shows it the way it shows a missing image:
  `[diagram: Line 4: …]`.
- New API command `diagram_draw {text, dark}` draws text that isn't a file yet (Dream previews).
- `app.js`: `renderMarkdown` already turns `![alt](path)` into `img.note-image`; also accept
  `![[x.fdraw]]` wiki embeds. `loadImages` passes `dark`. Dream previews: a proposal's preview
  contains the link to a file that doesn't exist yet, so the preview pane draws images whose
  `data-src` matches a diagram the proposal carries (`proposal.plan.diagram.file`) through
  `diagram_draw` instead.
- Diagrams get the `note-image` styling (max width of the column).

## 7. Diagrams Dream writes

### 7.1 The diagram in a plan

```rust
pub struct Diagram {
    pub file: String,     // link target relative to the note, "attachments/<name>.fdraw"
    pub caption: String,  // the image's alt text
    pub text: String,     // the .fdraw source
}
```

`Plan::Merge` and `Plan::Topic` get `#[serde(default)] diagram: Option<Diagram>`. New
`Plan::Diagram { note: String, diagram: Diagram }` and `Kind::Diagram` (`"diagram"`, parse also
`"draw"`). Old reports without the field still load.

### 7.2 Placement and naming

- File: `<the note's folder>/attachments/<note stem>.fdraw`, made free with `fsx::free_name` at apply
  time (the link in the note uses the final name, spaces as `%20` as capture does).
- Link: `![<caption>](attachments/<name>.fdraw)` on its own paragraph, inserted after the note's
  first `# ` heading line, and after the summary blockquote that directly follows it (topic notes).
  With no `# ` heading, at the top of the body (after front matter). FastMind inserts it; the model
  never writes the link.
- `.fdraw` joins `ATTACHMENT_EXT`, so a diagram whose note was archived or merged away is archived as
  an unlinked attachment, as images are. When a merge absorbs a note that had a diagram, the merged
  text keeps whatever links the model wrote (the merge skill says to keep images).

### 7.3 Language guidance for the model

New default skill `fastdraw-language.md`: FastDraw's language, condensed from FastDraw's `guide.md`
for a model that cannot run `fastdraw`: elements and styles of the built-in look, `at x,y`, sizes,
arrows and `style=`, labels, groups, legend, coordinates and spacing rules, and a short worked
example for each of the kinds Dream may draw (architecture, flowchart, process, state, layers,
timeline, tree, sequence). Rules: no `look` line, no icons from files, at most about a dozen boxes,
labels from the note's own words. Like every skill it is user-editable.

`skills::system` gains an `extra` skill name list (or a sibling function) so the merge, gather-topic,
gather-thread and draw-diagram prompts end with the language skill's body. The verdict cache key
already hashes the whole system prompt.

### 7.4 When a diagram is worth it (the judgment)

Stated in `draw-diagram.md` and, shortened, in `merge.md`, `gather-topic.md`, `gather-thread.md`:

- Worth it: the note describes **structure or movement** a reader has to hold in their head: parts
  and how they connect, steps with branches, states and transitions, layers, a sequence of messages,
  a timeline of more than a few events.
- Not worth it: a list, a single fact, a how-to with linear steps a numbered list says as well,
  opinions, records (meetings, daily notes, logs), anything where the diagram would only repeat the
  headings. "If a three-column table would say the same thing, no diagram."
- Draw only what the note says. One diagram per note.
- In merge/topic answers: optional fields `diagram` (fdraw text, `""` for none) and
  `diagram_caption`. Most answers have none.

### 7.5 Checking what the model drew

After any answer with a diagram, FastMind calls `diagram::errors`. If there are errors, the proposal
is held with its broken diagram and FastMind asks **once** more in the final diagram pass (§8.2),
after settle, so fixes never take requests from merges, topics, archive, links or settle (my call,
from the anti-goal in §1): the previous diagram, the errors, "fix only these" (schema: `{diagram}`),
using the same skill prompt; the proposal's plan and preview are updated with the fixed diagram. Still errors, or the budget is spent: the
diagram is dropped (the note proposal stays), and a model note in the report says so
("1 diagram dropped: Line 4: …"). Warnings are ignored. The retry is cached like any verdict (keyed by
the task key + "fix"), so a re-run night doesn't ask again.

## 8. The `diagram` kind (existing notes)

### 8.1 Candidates (statistics only)

A note is a candidate when all hold:

- not protected, not a record, not a template, not an AI session or thread note, not in the inbox,
  not touched in the last two days (the existing Dream rules);
- 250 words or more in the body;
- has no image or diagram link already (`![…](…)` or `![[…]]` to an image/`.fdraw`);
- not in a pending merge/topic/archive proposal this night;
- not skipped before with the same signature (existing skip memory), and no cached "not worth it"
  verdict for its current hash (the verdict cache handles this).

Ranked by a structure score: count of `->`/`→`/`=>`, numbered steps, words like *calls, sends,
depends on, flows, then, state, stage, layer, step, phase, before/after*, and `##` headings; ties by
most recently opened (usage) then most recently touched. The top `dream.diagram_notes` (new setting,
default **3**, `0` turns it off) are asked.

### 8.2 Budget and order

Diagram work (fixes from §7.5, then diagram questions) runs **last**, after settle. It has a
**reserve**: `min(3, model_requests)` requests are held back from the earlier steps (with the default
20, merges, topics, archive, links and settle share 17), and the diagram pass may use the reserve plus
whatever the earlier steps left. The user chose this so diagrams still run on a busy notebook, e.g. the
first night after upgrading, when the changed merge/topic prompts invalidate their cached verdicts.
Each candidate is one request (plus at most one fix). With `diagram_notes = 0` nothing is reserved.
`Judge` gets a way to hold and release the reserve (builder's choice of mechanism).

### 8.3 The question

Skill `draw-diagram` with the note in a delimited block (`ai::block`, up to 6,000 chars). Schema:

```json
{"worth": bool, "kind": string, "caption": string, "diagram": string, "reason": string}
```

`worth: false` → no proposal (cached by the note's hash, so not asked again until the note changes).
`worth: true` with a clean (or fixed) diagram → a proposal:

- title `Diagram for "<note title>"`, reason = model's reason, `notes = [note]`,
  `notes_removed = 0`, `bytes_freed = -(size of diagram + link line)`;
- `preview` = the note's text with the link inserted (§7.2), so Preview shows the note with its
  diagram;
- `sources = [(note, hash)]`: applies only while the note is unchanged, otherwise *out of date*.

### 8.4 Applying and undoing

One journal transaction: write the `.fdraw` (free name), rewrite the note with the link. Undo
restores both through the journal as for any proposal. Not in `auto_apply` by default (the user can
add `diagram`). For merge/topic proposals with a diagram, apply writes the `.fdraw` in the same
transaction, beside the kept/new note.

## 9. UI, CLI, settings, docs

- Dream list: `KIND_NAME.diagram = "Diagram"`, an icon; merge/topic cards with a diagram show
  "+ diagram" in the meta line.
- CLI: `fastmind dream` lists and applies the kind like others (generic over `Kind`); `dream apply
  --kind diagram` works through `Kind::parse`.
- Settings: `dream.diagram_notes` (default 3) in `DreamSettings` and the settings UI next to model
  requests.
- Docs: `features.md` (Dream draws diagrams; diagrams in notes), `reference.md` (setting, kind),
  `design.md` (Dream step 10: diagrams; the crate), README feature line. Licences: FastDraw (MIT),
  Phosphor Icons (MIT), the fonts (OFL) in a `LICENSES.md`/`licenses/` as FastPad does.

## 10. Testing

Fake models as in `tests/dream_ai.rs`:

- `diagram::draw`: a valid diagram gives an SVG with `@font-face`; an unknown shape gives
  `Line N: …`; look-path refusal for `//server/x.look`, `\\server\x.look`, `C:/x.look`, `C:x.look`,
  `/x.look`; `x.look` and a bundled name pass.
- `image` API on a `.fdraw` returns an SVG data URL; a broken one is an error with the line.
- Topic answer with a valid diagram → plan carries it, preview has the link after the summary;
  applying writes `attachments/<title>.fdraw`; undo removes it.
- Answer with a broken diagram → one fix request; fixed → kept; still broken → dropped, note
  proposal kept, model note says so.
- Diagram kind: a long structural note is asked, a short or list-only note is not, a note with an
  image is not; `worth:false` makes no proposal and isn't asked again next run; apply inserts the
  link after the heading and writes the file; editing the note makes it out of date;
  `diagram_notes = 0` asks nothing; diagram questions don't run when the budget is spent by earlier
  steps.
- Unlinked `.fdraw` is proposed for archive like an image.
- Old `latest.json` without `diagram` fields loads.

## 11. Left to the builder

The exact structure-score weights; the wording of the skills (within §7.3–7.4); how fonts are cached;
the Dream card icon; where in `run_in` the diagram step's code lives.
