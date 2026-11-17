---
name: gather-thread
description: Gathers AI coding sessions that continued one piece of work into a thread note.
---

# Gather AI sessions into a thread

These are notes made from the user's AI coding sessions (Claude Code, Codex) in one project.
Work often spans several sessions: one runs out of context, the next picks it up. Gather the
sessions that continued the same piece of work into one thread note, so the user can see the
whole story in one place. The session notes are archived and can be restored.

## Which sessions belong

Sessions belong together when they pursue the same goal: the same feature, bug, migration or
investigation, where a later session continues from an earlier one. Being in the same project
isn't enough. Leave out sessions about something else; if fewer than two belong, return an
empty `members` list.

## The thread note

- **title**: the piece of work, as the user would name it: "Winget publishing for FastCapture".
  Not the first prompt, no dates.
- **summary**: one sentence on where the work stands: what was achieved, or what's left.
- **body** (Markdown, without the title), with only the sections that apply:
  - `## Goal`: what the work set out to do, in a sentence or two.
  - `## How it went`: one bullet per session, in date order: "**2026-11-08**: …" saying what
    was tried, found or decided. Keep dead ends; they stop the user repeating them.
  - `## Where it ended`: the final state: merged, shipped, abandoned, or waiting on something.
  - `## Files and commands`: the files that changed and the commands that mattered, commands in
    fenced blocks, exactly as run.
  - `## Open`: what's left to do, as `- [ ]` tasks.
  Keep every finding, decision, file name, command and error message from the sessions. Add
  nothing they don't say. Never include secrets, tokens or passwords.
- **reason**: one plain sentence for the user: why these sessions are one thread.

## Diagram (optional)

Most thread notes need no diagram: leave `diagram` and `diagram_caption` as `""`. Draw one only
when the note describes structure or movement a reader has to hold in their head: parts and how
they connect, steps with branches, states and transitions, layers, a sequence of messages, a
timeline of more than a few events. Not for a list, a single fact, a how-to with steps in a line,
opinions or records, nor when it would only repeat the headings: if a three-column table would
say the same thing, no diagram. Draw only what the note says, in FastDraw (below), without a code
fence; `diagram_caption` says in a short phrase what it shows. FastMind puts it under the title.
