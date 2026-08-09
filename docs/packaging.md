# Local macOS packaging

These helpers create an unsigned app bundle for local dogfooding. They do not
create a DMG, sign or notarize the app, install `omp`, or bundle `omp`.

## Build the app bundle

From the repository root:

```sh
scripts/package_macos_app.sh
open dist/Pimiento.app
```

The script builds `pimiento-app` in release mode and assembles
`dist/Pimiento.app`. It rasterizes the checked-in
`assets/pimiento-icon.svg` into the required sizes, combines them into
`Contents/Resources/Pimiento.icns`, and declares that icon in the bundle's
`Info.plist`. Re-running it replaces the existing bundle.

## Install for the current user

```sh
scripts/install_macos_app.sh
open "$HOME/Applications/Pimiento.app"
```

The installer packages the current source, creates `~/Applications` when
needed, and replaces `~/Applications/Pimiento.app`. This avoids requiring
administrator privileges. Set `PIMIENTO_INSTALL_DIR` to choose another
installation directory.

Because the bundle is unsigned and intended only for local use, macOS may show
a Gatekeeper warning when it is moved to another machine.

## Linux

For personal dogfood, build a release binary and portable tarball with:

```sh
scripts/package_linux.sh
```

Outputs are written under `dist/pimiento-linux/`:

- `pimiento-app` — the unpacked release binary
- `README.txt` — runtime and launch instructions
- `pimiento-linux-<arch>.tar.gz` — those two files in a `pimiento/` directory

This tarball is the minimal E1 personal-dogfood path, not a system package.
AppImage / `.deb`, desktop integration, signing, and update delivery remain
deferred. That is an explicit waiver for personal dogfood only; a distributable
Linux release still requires one of those packaging formats plus platform QA.

The archive does not bundle system libraries. Build it on a compatible Linux
system, extract it, ensure `omp` is available on the login-shell `PATH`, and run
`./pimiento/pimiento-app`. Pimiento never bundles or installs `omp`.

## IME API dependency (E3)

The Enter-while-composing guard is blocked on a public gpui-component
composition API. The upstream ask is an `InputState::is_composing()` query or
equivalent submit-event flag; see `docs/protocol-notes.md` § “Wave E3 upstream
IME API ask”. No upstream fix is claimed yet.

## Windows

Windows packaging is optional and deferred by `PLAN.md`; no support claim is
made by these scripts.
