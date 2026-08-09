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

## API / reference (hard)

- Prefer pinned **Zed `gpui` / `gpui_platform`** and **gpui-component** under
  `~/.cargo/git/checkouts/` (Apache-2.0) for any UI/API question.
- Zed’s `ui`, `editor`, `terminal_view`, and `agent_ui` are **GPL-3.0-or-later**.
  **Patterns only, never copy source.**

## Depth recipe (Zed / gpui-component)

| Surface | Recipe |
|---------|--------|
| Docked (rail, inspector) | Tone (`sidebar` / `secondary`) + hairline `border` — **no shadow** |
| Composer band | Elevated island: `secondary`/`popover` fill + top hairline + **`shadow_md`/`lg` only on this band** (T4 float *feel*, still docked for list stability) |
| Floating (model picker, palette, About, slash) | `popover` + `border` + `radius`/`rounded_lg` + **`shadow_lg` / `shadow_xl`** over `overlay` |
| Selected rail row | `secondary` / `sidebar_accent` wash + `rounded_sm` — **no** primary accent bar / `border_l_2` |
| Tool / inset / action-group rows | `secondary` (PLAN “elevated”) + hairline border |

No purple glow, neon rings, or shadows on docked chrome (composer band is the sole docked exception). Prefer gpui-component
popover/dialog primitives when wiring is clean (placement + Wayland).

**Brand / status:** see `docs/parity-plan.md` §3 — paprika identity + ember action (never T4 Pi Pink); status = dot+label taxonomy. Shipped raw color tokens live only in `crates/pimiento-app/src/tokens.rs`; consume those tokens instead of adding ad hoc colors elsewhere.

## Doctrine (do not violate)

From `PLAN.md` §8 / `AGENTS.md`:

- Transcript is **semantic rows**, not chat bubbles: full-width; user = accent left border; assistant plain; tools inset elevated; thinking muted/italic/collapsed; errors danger-tinted.
- No dashboard theater, no permanent decorative panes, no gratuitous motion.
- Client projects OMP state only — never invent model/thinking/session truth.
- Prefer gpui-component primitives for overlays/popups.

## macOS chrome goals

1. **Toolbar anatomy (HIG):** leading = session/rail controls; center = live status (phase, ctx%, tps); trailing = drawers/overflow — **not** model/fast (those live on the composer band).
2. **8pt grid:** padding/gaps are multiples of 4/8 (`px(8.)`, `px(12.)`, `px(16.)`).
3. **Deference:** content (transcript) is the hero; chrome is quiet (`theme.muted` / borders), not a second information dump.
4. **Grouping:** separate status readouts from action buttons; insert visual gaps between groups.
5. **Density:** macOS body ~13pt (`text_sm` / `text_xs` for meta); avoid wall-of-ghost-buttons.
6. **Empty states:** one short sentence + primary action; no giant void without orientation.
7. **Copy affordances:** keep, but prefer hover/row-trailing ghost actions — don't make every row look like a button bar by default if a quieter pattern exists.

## Status strip rules

- `status_message` holds **OMP version / connection only** (or a transient notice like abort-arm). Do **not** also embed model + think + ctx there if composer/inspector already show them.
- Model + thinking + Fast live on the **composer band** (docked `secondary` + top hairline).
- Thinking control shows **only levels the active model supports** from `get_available_models[].thinking.efforts`, plus agent selectors `off` and `auto` when the model has a thinking config. If `thinking` is absent/`null`, hide or disable with reason ("no controllable thinking").
- Todos / Agents / Export / Rename / Sessions / Theme: trailing or palette (Cmd+K) when the bar overcrowds.

## Session rail (left)

- Depth: `sidebar` + `sidebar_border`; denser 8pt spacing.
- Group sessions by **workspace** (`session_cwd`); header = basename primary + muted path; per-workspace **`+`** adds a tab that connects with that cwd.
- Selected: wash + `rounded_sm` only — **no** `border_l_2` / primary accent bar.
- Hover **×** closes that index and `forget_session` from Pimiento `recent.json` — **never** delete OMP session files under `~/.omp`.
- Top chrome: compact **Workspace…** (directory picker → connect) + Hide/⌘B — not a New/Close/Hide prose wall.
- Prefer gpui-component `Separator`, `Tag`, `Switch` over walls of ghost text buttons.

## Context inspector (right, on demand)

PLAN §8: `[rail] [transcript] [right pane — Todos | Subagents | Session info]`.

- Quiet `sidebar` strip (tone + border only).
- Collapsible; Cmd+B = left rail, Cmd+J / palette toggles inspector.
- OpenCode-inspired sections, **OMP-honest only**:
  - **Session / Context** — cwd, model, thinking, ctx%, tps, phase
  - **Checklist** — only when `todoPhases` nonempty
  - **Agents** — subagent snapshots / subscription data
  - **Tools** — only when `dumpTools` names present in `get_state`
- **Do not** put Fast here (composer band owns it). **Do not** invent an LSP/MCP footer.
- Prefer `Accordion` / collapsible section headers; keep transcript the hero (inspector ~240–280px).

## Composer

- Band: model control → floating picker (`popover` + `shadow_lg`); Fast `Switch`; thinking when catalog has efforts; Attach + image chips; Send primary.
- **Roles (from omp config):** peek `modelRoles` + `modelTags` colors (built-in OMP palette). Click = `set_model`. **Set** = assign current model via `omp config set modelRoles` (merge; never hand-edit YAML).
- Image attachments only (v1): file picker + `ExternalPaths` drop; wire `{type, mimeType, data}` on Prompt/Steer/FollowUp.
- Short placeholder; put keymap hints in status/help or palette, not a novel in the placeholder.
- Fast Switch disabled when the model has no service-tier family (e.g. Cursor/Grok).

## Floating overlays

- Command palette / About: `overlay` scrim + `popover` + `shadow_xl`; focused search `Input`; Esc/arrows on an outer `capture_key_down` that wraps the overlay sibling.
- Clip long lists with `max_h` + `overflow_y_scrollbar` (inner scroll region if `on_click` must stay on the panel).

## Checklist before merge

- [ ] No duplicated model/think/ctx/fast labels across strip + composer + inspector
- [ ] Thinking options filtered to current model
- [ ] Docked = tone+border; floating = popover+shadow
- [ ] Light + dark still readable via theme tokens
- [ ] `cargo check -p pimiento-app` + app nextest green
- [ ] Note wire/UI surprises in `docs/protocol-notes.md`
