---
name: gather-topic
description: Decides whether scattered short notes make one subject and writes the topic note.
---

# Gather short notes into a topic

While the user sleeps, you look at short notes that sit near each other: fragments jotted down
over days or weeks that may all be about one subject. If they are, one well-organised topic note
replaces them (they're archived, and can be restored). A good topic note is worth more than its
parts: it's where the user goes to see everything they know about something.

## Which notes belong

Gather only notes genuinely about the same subject: the same thing, project, problem or
practice. Sharing a word or a tag isn't enough ("Postgres vacuum" and "Postgres backups" are
different subjects unless the notes connect them). Leave out any note that doesn't belong; if
fewer than two belong together, return an empty `members` list.

Records stay as they are: don't gather meeting notes, daily notes or logs, however alike.

## The topic note

- **title**: the subject as the user would search for it, two to six words, a noun phrase:
  "Sourdough starter", "Kubernetes health probes". No dates, no "Notes on".
- **summary**: one sentence saying what the notes together establish, not what they're about.
- **body** (Markdown, without the title):
  - Organise by sub-subject under `##` headings, in the order a reader needs them, not the
    order they were written. A short topic may need no headings at all.
  - When the subject changed over time (a decision revised, a value changed), tell it in date
    order within its section, with dates.
  - Put what's still to do or unresolved last, under `## Open`, keeping each task's `[ ]`.
  - Lose nothing: every fact, number, name, decision, link (`[[wikilinks]]` and URLs), image,
    code block, task and date from every note. Say each thing once.
  - Where notes disagree, say so with both dates. Add nothing that isn't in the notes.
- **reason**: one plain sentence for the user: why these belong together.

## Diagram (optional)

Most topic notes need no diagram: leave `diagram` and `diagram_caption` as `""`. Draw one only
when the note describes structure or movement a reader has to hold in their head: parts and how
they connect, steps with branches, states and transitions, layers, a sequence of messages, a
timeline of more than a few events. Not for a list, a single fact, a how-to with steps in a line,
opinions or records, nor when it would only repeat the headings: if a three-column table would
say the same thing, no diagram. Draw only what the note says, in FastDraw (below), without a code
fence; `diagram_caption` says in a short phrase what it shows. FastMind puts it under the title.

## Example

n1 (2026-10-23) "Readiness probe": "readiness failing = pod pulled from the service"
n2 (2026-10-25) "Liveness probe restarts": "liveness too strict → restart loops under load"
n3 (2026-10-30) "probe timeouts": "timeoutSeconds defaults to 1s!! slow /health fails it"

```json
{
  "members": ["n1", "n2", "n3"],
  "title": "Kubernetes health probes",
  "summary": "Readiness decides traffic, liveness decides restarts, and the 1-second default timeout makes slow health endpoints fail both.",
  "body": "## Readiness\n\nA failing readiness probe takes the pod out of the service; it isn't restarted.\n\n## Liveness\n\nA failing liveness probe restarts the pod. Too strict a probe causes restart loops under load.\n\n## Timeouts\n\n`timeoutSeconds` defaults to 1 second, so a slow `/health` endpoint fails the probe.",
  "reason": "All three are about how Kubernetes probes decide traffic and restarts.",
  "diagram": "", "diagram_caption": ""
}
```
