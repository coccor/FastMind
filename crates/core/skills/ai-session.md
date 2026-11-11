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

## When there's nothing to keep

Many sessions hold nothing worth a note: a question asked and abandoned, a request with no work
done, a quick lookup, a session that only ran a command. A session may open with the agent
restating what earlier sessions did (it's told at the start); that recap, a greeting or a "what's
next?" is not work. Then write no note (your answer says how). A notebook full of empty session
notes is worse than a missing one.

Never suggest next steps, causes or fixes the session didn't reach. The note records what
happened, not what could happen.

## Shape

- **Title**: the finding as a short claim, at most eight words: "Per-item archive index fixes
  undo", not "Archive work". The details belong in the summary. When nothing was found or
  decided, what was done.
- **Summary**: one sentence that answers the question on its own.
- **Body**: Markdown with only the sections that apply: `## Problem`, `## What was found`,
  `## Decision`, `## What worked` (commands in fenced blocks, exactly as run), `## Watch out`.
  Be brief and concrete; name files and functions.

Never include secrets, tokens, passwords or personal data; write `<redacted>` in their place.
