# Changesets

Drop a Markdown file in this directory to record a release-worthy change.
Each file is a single bump (patch / minor / major) plus a one-paragraph
summary; `porter version` aggregates them into the next release and a
single `CHANGELOG.md` entry, then deletes them.

```
---
bump: minor
---

Add the `attest` subcommand.
```

The cleanest way to add one is `porter add` — it prompts for the bump kind
and summary, generates a slug filename, and writes the file.

A few rules:

- One file per change. Don't squeeze unrelated changes into a single file.
- The bump category is the *user-visible* impact, not the diff size. A
  one-line bug fix that breaks compatibility is still `major`.
- Keep the summary tight. The summary is what lands in `CHANGELOG.md`
  verbatim; future you will read it as a release note, not a commit log.
