---
name: pimiento-ui-craft
description: >-
  Design and polish Pimiento's GPUI desktop UI using macOS HIG principles plus
  Pimiento doctrine (semantic transcript rows, OMP-authoritative state, no
  dashboard theater). Use when cleaning up the app chrome, toolbars, session
  rail, status strip, composer, dialogs, empty states, spacing, or visual
  hierarchy in crates/pimiento-app.
---

# Pimiento UI craft

Apply when changing Pimiento visuals or chrome. Companion skills (install globally):
`macos-native-ui`, `human-interface-guidelines`, `apple-design`.

## Doctrine (do not violate)

From `PLAN.md` §8 / `AGENTS.md`:

- Transcript is **semantic rows**, not chat bubbles: full-width; user = accent left border; assistant plain; tools inset elevated; thinking muted/italic/collapsed; errors danger-tinted.
- No dashboard theater, no permanent decorative panes, no gratuitous motion.
- Client projects OMP state only — never invent model/thinking/session truth.
- Prefer gpui-component primitives for overlays/popups.

## macOS chrome goals

1. **Toolbar anatomy (HIG):** leading = session/rail controls; center = live status (phase, ctx%, tps); trailing = actions (model, thinking, fast, drawers, overflow). Do not repeat the same fact in two places.
2. **8pt grid:** padding/gaps are multiples of 4/8 (`px(8.)`, `px(12.)`, `px(16.)`).
3. **Deference:** content (transcript) is the hero; chrome is quiet (`theme.muted` / borders), not a second information dump.
4. **Grouping:** separate status readouts from action buttons; insert visual gaps between groups.
5. **Density:** macOS body ~13pt (`text_sm` / `text_xs` for meta); avoid wall-of-ghost-buttons.
6. **Empty states:** one short sentence + primary action; no giant void without orientation.
7. **Copy affordances:** keep, but prefer hover/row-trailing ghost actions — don't make every row look like a button bar by default if a quieter pattern exists.

## Status strip rules

- `status_message` holds **OMP version / connection only** (or a transient notice like abort-arm). Do **not** also embed model + think + ctx there if buttons already show them.
- Model button shows current model id (shorten provider when obvious).
- Thinking control shows **only levels the active model supports** from `get_available_models[].thinking.efforts`, plus agent selectors `off` and `auto` when the model has a thinking config. If `thinking` is absent/`null`, hide or disable with reason ("no controllable thinking").
- Fast / Todos / Agents / Export / Rename / Sessions / Theme: trailing actions; collapse rarely used into palette (Cmd+K) when the bar overcrowds.

## Session rail (left)

- Group sessions by **workspace** (`session_cwd`); section headers use the directory basename (full path in tooltip/muted subtitle if needed).
- Within a workspace: compact rows with name + phase `Tag`/badge; active row selected style.
- New / Close / Hide are rail chrome, not a second title bar of prose buttons.
- Prefer gpui-component `Separator`, `Tag`, `Switch` over walls of ghost text buttons.

## Context inspector (right, on demand)

PLAN §8: `[rail] [transcript] [right pane — Todos | Subagents | Session info]`.

- Collapsible; Cmd+B = left rail, palette/More also toggles inspector (add Cmd+] if easy).
- OpenCode-inspired sections, **OMP-honest only**:
  - **Session / Context** — cwd, model, thinking, ctx%, tps, phase
  - **Fast** — `Switch` bound to `set_fast_mode` (show enabled vs active divergence in muted text)
  - **Checklist** — `todoPhases`
  - **Agents** — subagent snapshots / subscription data
  - **Tools** — `dumpTools` names if present in `get_state`; otherwise omit
  - **LSP / MCP** — only if the wire exposes them; otherwise a single muted note that rpc-ui does not publish status (do not invent)
- Prefer `Accordion` / collapsible section headers; keep transcript the hero (inspector ~240–280px).

## Composer

- Short placeholder; put keymap hints in status/help or palette, not a novel in the placeholder.
- Send is primary trailing control; keep secondary Enter (Cmd/Ctrl+Enter) behavior.

## Checklist before merge

- [ ] No duplicated model/think/ctx labels
- [ ] Thinking options filtered to current model
- [ ] Light + dark still readable via theme tokens
- [ ] `cargo check -p pimiento-app` + app nextest green
- [ ] Note wire/UI surprises in `docs/protocol-notes.md`
