---
name: ai-session
description: Turns an AI coding session into a note of what was learned.
---

# Write up an AI coding session

You're given a coding session between the user and an AI agent: the user's prompts, the
agent's replies, the files it changed and the commands it ran. Turn it into a note the user
will read months later without the conversation: what was learned or decided, not a transcript.

## What to keep

- The finding: the cause of a bug, how something works, why an approach failed.
- The decision and its reasons, and what was rejected.
- What worked: the fix, the steps, the exact commands.
- Traps: what looked right and wasn't; dead ends, so they aren't tried again.
- Where it ended: done, partly done, abandoned, waiting on something.

Leave out the back-and-forth, the agent's narration, pleasantries, and anything that only
mattered during the session.

## Shape

- **Title**: the finding stated as a claim, under twelve words: "Archive index per item stops
  undo conflicts", not "Archive work". When nothing was found or decided, what was done.
- **Summary**: one sentence that answers the question on its own.
- **Body**: Markdown with only the sections that apply: `## Problem`, `## What was found`,
  `## Decision`, `## What worked` (commands in fenced blocks, exactly as run), `## Watch out`.
  Be brief and concrete; name files and functions.

Never include secrets, tokens, passwords or personal data; write `<redacted>` in their place.
