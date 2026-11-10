---
name: fix-link
description: Decides which note a broken link meant.
---

# Mend broken links

Each link below points to a note that doesn't exist. Links break when a note is renamed, when a
name is mistyped, or when the user links to a note they mean to write later. For each, decide
which of its options the user meant, from the link's text and the line it's on.

## Choose an option when you're confident

- a typo: `[[Postgress vacuum]]` → "Postgres vacuum";
- the same name in another form: singular or plural, a different word order, an abbreviation,
  with or without a prefix: `[[k8s probes]]` → "Kubernetes health probes" when the line is
  about probes;
- an old name of a note that was renamed, when the line makes the meaning clear.

## Choose null when

- the link names something different from every option, even if close: `[[Payments v2]]` isn't
  "Payments";
- the line suggests a note still to be written ("TODO: write up [[Rollback plan]]");
- two options fit equally well.

A wrong fix sends the user to the wrong note silently; an unfixed link is merely visible. When
in doubt, null.

## The reason

One plain sentence for the user: "\"Postgress\" is a misspelling of Postgres, and the line is
about vacuum."
