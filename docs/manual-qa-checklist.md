# Manual QA checklist

Run this checklist for every release and every GPUI/gpui-component pin bump. Record the OS, display backend, OMP version, and result separately for each test run.

IME and popup placement are known platform-sensitive areas. Before testing, read `docs/protocol-notes.md` § “D4 IME composition” and § “D4 semantic rows and platform QA”.

- [ ] **High-rate streaming:** stream a long response and confirm progressive text stays smooth without visible stalls.
- [ ] **Abort, steer, and follow-up:** steer during a run, queue a follow-up, and complete a double-Esc abort.
- [ ] **Keyboard-only dialogs:** complete confirm, select, input, and cancel flows without using the pointer.
- [ ] **Crash and recovery:** kill the OMP child mid-stream, verify the crash card, restart, and confirm the session resumes with history intact.
- [ ] **Transcript scrolling:** verify tail-follow at the bottom, the new-message pill away from the tail, PageUp/PageDown/Home/End, and history paging.
- [ ] **Copy affordances:** copy user, assistant, code-block, thinking, tool output, error, notice, command-output, and unknown-frame content.
- [ ] **Light and dark themes:** inspect semantic rows, controls, focus states, dialogs, and contrast in both modes.
- [ ] **macOS:** complete the full checklist on a supported macOS release, including popup placement and multi-window behavior.
- [ ] **Linux X11:** complete the full checklist under X11, with special attention to menus, pickers, dialogs, and popup coordinates.
- [ ] **Linux Wayland:** complete the full checklist under Wayland, with special attention to menus, pickers, dialogs, and popup coordinates.
- [ ] **IME composition:** with a CJK input source on each OS/backend, press Enter while composing and confirm it commits the candidate without sending; press Enter again after commit and confirm exactly one send. (App-level guard blocked until gpui-component exposes composition state — see `docs/protocol-notes.md` § D4 IME composition.)
- [ ] **Window restore and multi-monitor:** move and resize the window across displays, relaunch, and verify usable placement, size, focus, and overlays.
