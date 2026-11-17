---
name: draw-diagram
description: Decides whether a note is worth a diagram and draws it.
---

# Draw a diagram for a note

While the user sleeps, you read a long note and decide whether a diagram would help a reader
understand it. If so, you draw one; it goes into the note under its title, with a caption. A
good diagram saves the reader holding the note's structure in their head. A needless one is
clutter, so most notes get none.

## When a diagram is worth it

Worth it: the note describes **structure or movement** a reader has to hold in their head:

- parts and how they connect (a system, a setup, who talks to whom);
- steps with branches: questions and where each answer leads;
- states and the events that move between them;
- layers that build on each other;
- a sequence of messages between parties;
- a timeline of more than a few events.

Not worth it:

- a list, a single fact, opinions;
- a how-to whose steps run in a line, which a numbered list says as well;
- records: meetings, daily notes, logs;
- anything where the diagram would only repeat the headings.

If a three-column table would say the same thing, no diagram. When in doubt, none.

## Drawing it

- Draw only what the note says, in its words: no part, step or arrow it doesn't mention.
- One diagram, of the kind that fits what the reader has to understand: the parts
  (architecture), decisions (flowchart), steps in order (process), states (state), levels
  (layers), events in time (timeline), a hierarchy (tree), messages (sequence).
- Keep it small: the dozen boxes that matter, not everything.

## The answer

- **worth**: `true` when the note is worth a diagram, else `false`.
- **kind**: the kind you drew (`architecture`, `flowchart`, `process`, `state`, `layers`,
  `timeline`, `tree`, `sequence`), or `""`.
- **caption**: a short phrase saying what the diagram shows, in the note's words: "How a
  login reaches the database". `""` when not worth it.
- **diagram**: the FastDraw text, without a code fence. `""` when not worth it.
- **reason**: one plain sentence for the user, specific to this note: why it helps, or why the
  note reads well as it is.

## Examples

A note on how an article goes from draft to published, with review sending it back:

```json
{
  "worth": true,
  "kind": "state",
  "caption": "How an article moves from draft to published",
  "diagram": "start: ellipse at 40,168 16x16 fill=ink stroke=none\ndraft: node \"Draft\" at 120,136 160x80 radius=8\nreview: node \"In review\" at 344,136 160x80 radius=8\ndone: focal \"Published\" at 568,136 160x80 radius=8\nstart -> draft\ndraft -> review \"submit\"\nreview -> done \"approve\" style=primary\nreview.top -> draft.top \"reject\" style=return route=curve\nlegend at 40,240 node=\"State\" focal=\"Outcome\" primary=\"Main path\" return=\"Back a step\"",
  "reason": "The note describes three states and a step back from review; a picture shows the loop at a glance."
}
```

A note listing the user's favourite cookbooks with a line on each:

```json
{"worth": false, "kind": "", "caption": "", "diagram": "", "reason": "A list of books; a diagram would only repeat it."}
```
