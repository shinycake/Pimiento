# App icon assets

Masters follow Apple HIG ([App Icons](https://developer.apple.com/design/human-interface-guidelines/app-icons))
and the same layout Zed uses under `crates/zed/resources/`:

| File | Size | Role |
|------|------|------|
| `app-icon.png` | 512×512 | 1× master (Dock / About / tooling) |
| `app-icon@2x.png` | 1024×1024 | 2× / marketing master |

## Rules

- **Full-bleed square** artwork — do **not** pre-bake the macOS squircle. The system
  applies continuous corner masking (and Liquid Glass effects on newer OS versions).
- Opaque sRGB PNG. Prefer a simple centered silhouette that stays legible at 16px.
- No text, screenshots, or Apple hardware replicas.
- Brand: paprika / ember pepper motif (`#C45C26` family).

`scripts/package_macos_app.sh` rasterizes these into `Pimiento.icns` and sets
`CFBundleIconFile` (name without extension) in `Info.plist`.
