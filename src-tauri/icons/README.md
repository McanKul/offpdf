# App icons

Tauri's bundler expects the icon files referenced in `tauri.conf.json`:

```
icons/32x32.png
icons/128x128.png
icons/128x128@2x.png
icons/icon.icns   (macOS)
icons/icon.ico    (Windows)
```

The canonical SVG artwork and its 1024x1024 launcher PNG live in `source/`.
Generate all platform files from that source with:

```bash
node scripts/gen-icon.mjs
npm run tauri icon scripts/icon-src.png
```

The Tauri command writes all required sizes and formats into this folder. It is
only needed for `tauri build` (release bundles); `tauri dev` runs without them.
