# Quiet Pepper motion and visual QA

This is the Wave U5 manual review record for the Quiet Pepper Console. Compare
Pimiento with T4 lane-b references for information hierarchy and calm density,
not color, branding, component geometry, or Electron chrome. Pimiento must
remain recognizably paprika/ember and GPUI-native.

## Motion recommendations

Use three duration tokens rather than one-off values:

| Token | Duration | Intended use |
|---|---:|---|
| `motion_fast` | 120 ms | Hover, pressed, focus, and small status changes |
| `motion_standard` | 180 ms | Expand/collapse, picker, and ordinary chrome transitions |
| `motion_deliberate` | 240 ms | Dialog or larger attention-surface entrance/exit |

Prefer ease-out for entrances and state changes. Theme flips, resizing, rail
dragging, transcript streaming, and rapidly updating counters should not
animate. Motion must explain a relationship or state change; it must not add
ambient decoration.

Reduced-motion policy:

- Honor the platform preference when GPUI exposes it; until then, provide no
  essential information through motion alone.
- Disable status pulses, smooth scrolling, animated reordering, and
  expand/collapse interpolation.
- Apply the final state immediately. Keep dot+label status, focus rings, and
  hierarchy unchanged.
- Never use continuous glow, shimmer, bounce, or looping progress decoration.

These values are recommendations until shared motion constants and the
platform-preference path are implemented. Raw color tokens already ship in
`crates/pimiento-app/src/tokens.rs`.

## Light/dark structural checklist

Run every item in both light and dark. Judge structure against T4; do not tune
Pimiento toward T4's colors.

- [ ] Transcript remains the visual hero; rail, toolbar, inspector, and composer
  do not compete with it.
- [ ] Primary action is unambiguous; secondary actions and metadata stay quiet.
- [ ] Model, thinking, Fast, context, and phase facts are not needlessly
  duplicated across chrome.
- [ ] Docked surfaces read through tone and hairlines; only the composer band is
  elevated, while true overlays clearly float above it.
- [ ] Semantic transcript rows remain distinct without turning into chat
  bubbles: user accent, plain assistant, inset tools, muted thinking, danger
  errors.
- [ ] Status uses dot plus label and never color alone.
- [ ] Selected rail rows use a wash and radius, not a decorative accent stripe.
- [ ] Tool/code content can use the pane width while prose keeps a readable
  measure.
- [ ] Focus, disabled, hover, warning, danger, and diff states retain the same
  hierarchy in both themes.
- [ ] Empty states orient briefly without dashboard theater or decorative void
  filling.

## Screenshot checklist

Capture matching light and dark screenshots at a consistent window size. Record
commit, OS, display backend, theme, and exact `omp --version` beside the set.

- [ ] **Streaming:** active assistant stream, working status pill, tail behavior,
  and Steer/Follow-up hierarchy visible.
- [ ] **Ask:** blocking ask/approval card, keyboard hints, composer dim/disable,
  and non-color-only attention state visible.
- [ ] **Tool expanded:** summary, status/duration, bounded mono output, copy, and
  elision treatment visible.
- [ ] **Composer island:** model, filtered thinking, Fast, Attach, context, and
  primary action visible without toolbar duplication.
- [ ] **Status pills:** idle, working, awaiting-input/approval, busy, and error
  examples captured from real projected states where practical; do not
  fabricate runtime state for a screenshot.

Store reviewed evidence under `docs/sh-artifacts/` or link large external
artifacts from an artifact README. A screenshot checklist is not green until
both themes have been inspected; screenshots alone do not satisfy behavioral
SH proofs.
