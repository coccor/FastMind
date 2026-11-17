---
name: merge
description: Decides which notes are duplicates and writes the merged note.
---

# Merge duplicate notes

While the user sleeps, you look at notes that might be duplicates: the same thing captured
twice, a note and its later rewrite, or two notes that say substantially the same thing. Merged
notes are what makes the notebook smaller and search sharper, but a wrong merge loses the
distinction between two things the user kept apart. Be sure.

## Which notes are duplicates

Duplicates cover the same subject **and** say mostly the same thing:

- the same fact, fix or idea captured twice, in different words;
- a note and a later, fuller version of it;
- a link saved twice, with or without comments.

Not duplicates, however alike they read:

- notes from the same template: two meetings, two daily notes, two recipes;
- two events on the same subject: last week's incident and this week's;
- a question and a separate note that answers it, a plan and its result, a general note and a
  specific case of it: these are related, and the user may want both;
- notes about different instances: two services, two customers, two versions.

When in doubt, leave them apart.

## Which one to keep

The note other notes link to most (`linked_from`), else the most complete one, else the newest.
The kept note keeps its title.

## Writing the merged note

- Start with `# ` and the kept note's title.
- Keep the kept note's structure and voice, and fold what the others add **into the right
  places**: a new step among the steps, a new fact beside the related one. Don't append "From
  the other note" sections.
- Lose nothing: every fact, number, name, decision, link (`[[wikilinks]]` and URLs), image,
  code block, task (with its `[ ]` or `[x]` state) and date from every note.
- Say each thing once. Where notes disagree, or something changed over time, keep both with
  their dates: "Password changed to `lime-otter-7` (2026-11-10; was `orange-falcon-42`)."
- Add nothing that isn't in the notes.

## Diagram (optional)

Most merged notes need no diagram: leave `diagram` and `diagram_caption` as `""`. Draw one only
when the note describes structure or movement a reader has to hold in their head: parts and how
they connect, steps with branches, states and transitions, layers, a sequence of messages, a
timeline of more than a few events. Not for a list, a single fact, a how-to with steps in a line,
opinions or records, nor when it would only repeat the headings: if a three-column table would
say the same thing, no diagram. Draw only what the note says, in FastDraw (below), without a code
fence; `diagram_caption` says in a short phrase what it shows. FastMind puts it under the title.

## The reason

One plain sentence the user will read in the morning, specific to these notes: "Both record
the conference wifi password; the second adds where it came from." Not "These notes are
similar."

## Example

n1 "Docker build cache" (linked from 2): "Copy package.json and run npm ci before copying the
source, so code changes don't bust the dependency layer."
n2 "docker layer caching": "copy deps manifest first then install, then COPY . — keeps the
cache. also BuildKit --mount=type=cache for npm cache"

```json
{"groups": [{
  "keep": "n1", "absorb": ["n2"],
  "reason": "Both describe ordering a Dockerfile to keep the dependency layer cached; the second adds BuildKit cache mounts.",
  "text": "# Docker build cache\n\nCopy `package.json` and run `npm ci` before copying the source, so code changes don't bust the dependency layer.\n\nBuildKit cache mounts (`--mount=type=cache`) also keep the npm cache between builds.",
  "diagram": "", "diagram_caption": ""
}]}
```
