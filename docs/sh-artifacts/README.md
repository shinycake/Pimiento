# SH artifact storage

Store small, reviewed Self-Host Gate evidence here. The canonical checklist and
status table are in `docs/sh-proofs.md`.

Use one directory per run:

```text
sh-N-<platform>-YYYY-MM-DD/
  README.md
  commands.txt
  screenshot.png
```

Each run README must identify the commit, OS/version, Linux display backend when
applicable, exact `omp --version`, scenario steps, result, and artifact
provenance. Prefer links plus checksums for large screen recordings or session
exports rather than committing them.

Never add credentials, API keys, private repository content, or raw session
exports before review and redaction. The presence of a directory or placeholder
does not mark a proof complete.
