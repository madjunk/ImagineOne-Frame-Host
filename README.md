# ImagineOne Application Frame Host (desktop app)

The Windows desktop app for the ImagineOne web application ("Imagine One IT", and each customer's
branded copy). It hosts the web app in its own window with no browser chrome, locks navigation to the
install's own server, verifies the configured company slug against the server at every start, and adds
a ☰ menu whose Administration section appears only for admins. Implements
`frame-host-api-reference.md`; the server needs no changes.

Built with **Tauri 2**: a small Rust program plus the Edge WebView2 that is part of Windows 10/11. The
installer is about 2 MB; the Electron version it replaces was about 112 MB.

## What it does

- **Window:** compact (480 × 700) for the organisation step and the sign-in page, full size once
  signed in (size and position remembered). Closing it keeps the app in the tray; Quit (☰ menu or
  tray) really quits.
- **Title bar:** our own glass title bar takes the tenant's branding (logo, name, colours) and, when
  signed in, continues the web app's page colour or the tenant's "Desktop Title Bar" colours.
- **Boot:**
  1. `GET /api/public/deployment` checks the server can be reached.
  2. `GET /api/public/tenant/:slug` is the install lock, when a company is configured.
  3. The page reports whether someone is still signed in, which decides between the app, the sign-in page and the organisation step.
- **Navigation:** only the configured server loads inside. Google / Microsoft sign-in runs in a small
  popup that shares the app's cookies and hands back to the app. Everything else (other sites,
  `mailto:`, `tel:`) opens in the default browser or app. Downloads are shown in Explorer.
- **Identifies itself:** the User-Agent carries `ImagineOneFrame/<version>`, so the server lets it past
  the "install the app" (PWA) gate. Its own API calls also send `X-ImagineOne-Frame`.
- **Remember me:** the web app's sign-in page uses `window.imagineOneFrame.credentials` (same API as
  the Electron preload). The password is kept with Windows data protection (DPAPI), per organisation,
  for this Windows user only.
- **Frame settings** (server address, company slug): for admins. They can also be opened from the
  "Configuration problem" screen, where nobody is signed in.
- **Updates:** from GitHub releases, signed with our update key; the app asks before installing.

## Replacing the Electron version

- Same product name, and the same settings file (`%APPDATA%\Imagine One IT\frame-config.json`, or
  `%APPDATA%\ImagineOne\<slug>\` for a customer build). So the new installer finds the Electron app,
  closes and uninstalls it, and keeps its server and company.
- Publish also uploads `latest.yml` / `<slug>.yml`, so Electron installs update themselves to this
  version.
- People sign in once more: the Electron app's cookies and saved passwords can't be read by this one.

## Build and release (ImagineOne Studio, or Desktop.ps1)

From ImagineOne Studio: **Desktop app** page (standard installer) and **Customers › Build installer**.
Or `Desktop.cmd` / PowerShell:

```powershell
.\Desktop.ps1 Setup                          # one time: Rust + the C++ build tools
.\Desktop.ps1 Dev [-Server https://app.imagineone.it]
.\Desktop.ps1 Build [-Publish]                 # dist\ImagineOne-Setup.exe (+ .sig, update.json, latest.yml)
.\Desktop.ps1 Customer -Customer nefco [-Publish]   # ..\customers\nefco\dist\nefco-Setup.exe
.\Desktop.ps1 Bump                            # raise the version before publishing an update
.\Desktop.ps1 Publish [-Customer nefco]       # upload the last build to the draft GitHub release v<version>
```

- **Standard installer:** applies `resources\frame-config.json` on first run (Studio: Desktop app ›
  Installer config).
- **Customer build:** uses `..\customers\<slug>\profile.json` (name, server, release repo) and the
  customer's icons.
- **Publish:** needs `$env:GH_TOKEN` with contents: write on the release repo
  (`madjunk/ImagineOne-Frame-Host` unless the customer profile says otherwise). It uploads the installer
  and both update feeds to the draft release `v<version>`; publish the draft on GitHub to make it
  live.
- **Download link:** the web app's `/download` page links to
  `releases/latest/download/ImagineOne-Setup.exe`, and that name is unchanged.
- **Update key:** builds are signed with `%USERPROFILE%\.tauri\i1-phone.key`, the same key as I1 Phone
  and the console. Never make a new one while apps are installed: copy it from the backup.

## Layout

```
Desktop.ps1 / Desktop.cmd   setup, run, build, customer build, bump, publish
src/renderer/               title bar + organisation / status / settings screens (React);
                            tauri-bridge.ts gives it window.frameHost in the Tauri app
src/shared/types.ts         the state the title bar draws (FrameState)
tauri/
  vite.ui.config.ts         npm run build:ui -> tauri/ui-dist (the title bar for the app)
  src-tauri/src/lib.rs      window (title bar + app webviews), boot, branding, menu, tray, sign-in popup,
                            saved passwords, updates; APP_JS = window.imagineOneFrame for the web app
  src-tauri/windows/        installer hooks (replace the Electron app)
src/main, src/preload       the old Electron version (no longer built; kept for reference)
```
