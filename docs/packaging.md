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
`dist/Pimiento.app`. Re-running it replaces the existing bundle.

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
