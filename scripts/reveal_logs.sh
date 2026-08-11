#!/bin/sh
# Reveal Pimiento's local home folder without reading or printing its contents.
set -eu

PIMIENTO_HOME="${PIMIENTO_HOME:-$HOME/.pimiento}"
mkdir -p "$PIMIENTO_HOME"

case "$(uname -s)" in
    Darwin)
        open "$PIMIENTO_HOME"
        ;;
    Linux)
        if command -v xdg-open >/dev/null 2>&1; then
            xdg-open "$PIMIENTO_HOME"
        else
            printf '%s\n' "xdg-open is required to reveal the Pimiento home folder" >&2
            exit 1
        fi
        ;;
    *)
        printf '%s\n' "Reveal logs is unsupported on this platform" >&2
        exit 1
        ;;
esac
