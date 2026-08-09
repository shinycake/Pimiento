# Self-Host Gate proof record

This is the evidence index for `PLAN.md` SH-1…SH-6. A checklist row is complete
only when a human has run the scenario and linked a reviewable artifact. An
available environment, passing unit tests, or an empty artifact slot is not a
proof.

Current status:

- **Linux:** environment ready / pending live dogfood
- **macOS:** pending user machine
- **Completed SH proofs:** none

| Gate | Required proof | Linux | macOS | Artifact links / notes |
|---|---|---|---|---|
| SH-1 | Pimiento drives the code-block-copy feature loop: failing build visible, fix, green build/tests, mid-run steer, rebuild/relaunch | Environment ready / pending live dogfood | Pending user machine | Pending |
| SH-2 | Answer an `ask` dialog keyboard-only during a run | Environment ready / pending live dogfood | Pending user machine | Pending |
| SH-3 | Kill the OMP child mid-stream; crash card, Restart, and resumed history all work | Environment ready / pending live dogfood | Pending user machine | Pending |
| SH-4 | Quit Pimiento mid-run; relaunch, resume the same pointer, and continue | Environment ready / pending live dogfood | Pending user machine | Pending |
| SH-5 | Render and interact with a build producing more than 1 MiB output without a UI stall; show elision/chunking behavior | Environment ready / pending live dogfood | Pending user machine | Pending |
| SH-6 | Complete the full loop on both platforms; record Linux backend (`X11` or `Wayland`) | Environment ready / pending live dogfood | Pending user machine | Cross-platform gate remains open until both sides are evidenced |

## Recording a proof

1. Create a subdirectory under `docs/sh-artifacts/` named
   `sh-N-<platform>-YYYY-MM-DD/`.
2. Add a short `README.md` recording commit, OS/version, display backend, exact
   `omp --version`, commands/actions, expected result, actual result, and any
   redactions.
3. Put compact text evidence in that directory. Large videos or session exports
   may be stored outside Git; add a stable link and checksum to the artifact
   README instead.
4. Link the artifact directory from the table above and change a status to
   **Complete** only after the evidence has been reviewed.

Do not commit credentials, session secrets, provider payloads, private source,
or unreviewed OMP session exports. Redact first and state what was removed.
