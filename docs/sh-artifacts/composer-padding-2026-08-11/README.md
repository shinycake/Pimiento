# Composer padding / grow / image preview — 2026-08-11

Commit: tip of `cursor/composer-paste-grow-padding-459e`
OS: Linux (Xfce, `DISPLAY=:1`)
`omp --version`: 17.2.12

| File | Shows |
|------|-------|
| `chat-transcript-padding.png` | Populated transcript with horizontal gutters (accent rail inset from column edge) |
| `chat-composer-grown.png` | Multi-line draft; composer chrome expands with `auto_grow` |
| `chat-composer-image-preview.png` | Clipboard image paste → 56px thumbnail + label/dims above the field |
| `chat-overview.png` | Full window with transcript + grown composer |

Notes:
- GPUI `list` ignores horizontal padding on the list element; gutters live on each row.
- Ctrl/Cmd+V is handled via `cx.intercept_keystrokes` so Input's Paste binding does not swallow images/paths.
