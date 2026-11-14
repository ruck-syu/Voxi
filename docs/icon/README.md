# App icon

`appicon.swift` draws `voxi-icon-1024.png`: a listening face (two eyes and a
smile drawn as a sound wave) on a plain paper tile sized to Apple's icon grid.
It also draws the menu bar icon, `app/src-tauri/icons/tray.png`. The website's
`website/assets/favicon.svg` is the same face; keep the two in step by hand.

To change the icon, edit the script, then from the repo root:

```bash
swift docs/icon/appicon.swift docs/icon/voxi-icon-1024.png app/src-tauri/icons/tray.png
sips -z 256 256 docs/icon/voxi-icon-1024.png --out docs/icon/voxi-icon-256.png
```

```bash
cd app && npx tauri icon ../docs/icon/voxi-icon-1024.png
```

`tauri icon` also writes Android and iOS icons; Voxi is Mac only, so delete
`app/src-tauri/icons/android`, `app/src-tauri/icons/ios` and `app/src-tauri/icons/64x64.png` afterwards.
