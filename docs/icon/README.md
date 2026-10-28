# App icon

`appicon.swift` draws `voxi-icon-1024.png`: the website's folder-and-spark mark
(`website/assets/favicon.svg`) on an index-card tile sized to Apple's icon grid.

To change the icon, edit the script, then from the repo root:

```bash
swift docs/icon/appicon.swift docs/icon/voxi-icon-1024.png
```

```bash
cd app && npx tauri icon ../docs/icon/voxi-icon-1024.png
```

`tauri icon` also writes Android and iOS icons; Voxi is Mac only, so delete
`app/src-tauri/icons/android` and `app/src-tauri/icons/ios` afterwards.
