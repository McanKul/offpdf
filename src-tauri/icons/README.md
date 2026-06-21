# App icons

Tauri's bundler expects the icon files referenced in `tauri.conf.json`:

```
icons/32x32.png
icons/128x128.png
icons/128x128@2x.png
icons/icon.icns   (macOS)
icons/icon.ico    (Windows)
```

Generate them from a single 1024x1024 PNG with the Tauri CLI:

```bash
npm run tauri icon path/to/source-1024.png
```

This command writes all required sizes/formats into this folder. It is only
needed for `tauri build` (release bundles); `tauri dev` runs without them.
