# ImagineOne Application Frame Host

Native desktop shell (Electron) for the ImagineOne web application. Implements
`frame-host-api-reference.md`: it hosts the existing web app in a sandboxed view
with no browser chrome, locks navigation to the install's own domain, verifies
the configured company slug against the server at every start, and exposes a
single native menu whose Administration section appears only for `admin` users.
No application code changes are required — everything it needs is already live
on the server.

## Stack

- Electron 44 (Chromium + Node) · electron-vite · React 19 + TypeScript
- electron-builder → Windows NSIS installer, macOS DMG
- electron-updater → auto-update from GitHub Releases (configurable)

## Layout

```
src/
  main/        Electron main process
    index.ts     window, app view, boot sequence, actions, tray, updater, IPC
    menu.ts      native menu (role-aware Administration section)
    api.ts       calls to /api/public/*, /api/auth/* on the app's cookie session
    config.ts    per-install config (server URL + company slug), JSON on disk
  preload/     the only bridge between the title-bar renderer and main
  renderer/    React title bar + boot/error/settings overlays (NOT the web app)
  shared/      types shared by all three
build/         icons, macOS entitlements (replace placeholder icons)
resources/     tray icon, optional frame-config.json packaged defaults
electron-builder.yml
```

How the window is composed: the renderer draws a 40 px title bar; the web app
lives in a separate `WebContentsView` (its own `persist:imagineone` cookie
partition, no preload, fully sandboxed) positioned below it. The server's
`auth_token` cookie persists in that partition across restarts, so the frame
never handles the token itself.

## Boot sequence

1. `GET /api/public/deployment` — reachability check. Fails → configuration screen.
2. If a company slug is configured: `GET /api/public/tenant/:slug`. 404 or slug
   mismatch → configuration screen (this is the install lock). Otherwise the
   response's `appTitle`, `logoUrl` and `primaryColor` brand the title bar.
3. Load the app; `GET /api/auth/me` on every navigation keeps the user/role
   current and rebuilds the native menu.

## Navigation scope

`will-navigate` and `setWindowOpenHandler` cancel anything whose host differs
from the configured server URL and hand it to the OS default handler
(`shell.openExternal`). `mailto:`/`tel:` go to the OS too. Downloads use the
normal save dialog and are revealed in the file manager when complete.

## Configuration

Runtime config is a JSON file in the user's app-data folder:

- Windows: `%APPDATA%\ImagineOne\frame-config.json`
- macOS: `~/Library/Application Support/ImagineOne/frame-config.json`

```json
{ "baseUrl": "https://app.imagineone.it", "tenantSlug": "" }
```

Users can edit it from the ⚙ button / "Frame settings…" menu item. To ship a
**customer-locked installer**, put the same JSON in `resources/frame-config.json`
before building; it is applied on first run.

## Develop

```bash
npm install
npm run dev          # hot-reloading renderer + main
npm run typecheck
```

`npm run dev` opens DevTools under View. In dev, the frame reads
`./frame-config.json` from the project root on first run if present.

## Build installers

```bash
npm run dist:win     # dist/ImagineOne-Setup-<version>.exe   (build on Windows)
npm run dist:mac     # dist/ImagineOne-<version>-universal.dmg (build on macOS)
```

Cross-building is limited: build each platform on that platform (or in CI —
a GitHub Actions matrix with `windows-latest` and `macos-latest` is the usual
setup).

### Signing / notarization (needed before distributing)

- **Windows**: an Authenticode certificate. Set `WIN_CSC_LINK` (base64 .pfx) and
  `WIN_CSC_KEY_PASSWORD`, or configure `azureSignOptions` in
  `electron-builder.yml`. Unsigned builds trigger SmartScreen.
- **macOS**: a *Developer ID Application* certificate. Set `CSC_LINK` /
  `CSC_KEY_PASSWORD`, then `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`,
  `APPLE_TEAM_ID` and flip `notarize: true` in `electron-builder.yml`.

### Auto-update

`publish` in `electron-builder.yml` points at GitHub Releases
(`madjunk/ImagineOne-Frame-Host` — change to the real repo). Publishing a release
with the installer assets + `latest.yml`/`latest-mac.yml` (electron-builder
produces them with `--publish always`) is all the app needs; it checks 10 s after
launch and on "Check for updates…". Signed builds are required for macOS
updates to install.

## Replacing placeholder artwork

`build/icon.png` (1024²), `build/icon.ico`, and `resources/tray.png` are
generated placeholders (blue square with a "1"). Drop in real artwork with the
same names.

## Notes for the child-instance work

- Admin menu paths (`/admin/settings`, `/admin/users`, `/admin/software-update`)
  are in `src/main/menu.ts` — adjust to the real routes.
- `selfHostedConsoleUrl` from `/api/auth/me`, when non-empty, adds an
  "Open deployment console" item under Administration.
- Sign out calls `POST /api/auth/logout`, clears the partition's cookies, and
  navigates to `/login`.

## Publishing a release (what the app's /download page links to)

The web app's `/download` page links to
`https://github.com/madjunk/ImagineOne-Frame-Host/releases/latest/download/ImagineOne-Setup.exe`
(override with `DESKTOP_APP_DOWNLOAD_URL` on the server). The frame's
auto-updater reads `latest.yml` from the same release. So one release = both.

1. Create the public repo `madjunk/ImagineOne-Frame-Host` once (it can hold
   this source, or stay empty and only carry releases).
2. Bump `version` in `package.json`.
3. Build and publish in one go — needs a GitHub token with `contents: write`
   on that repo:
   ```powershell
   $env:GH_TOKEN = "<token>"
   npm run dist:win -- --publish always
   ```
   electron-builder uploads `ImagineOne-Setup.exe`, the `.blockmap`, and
   `latest.yml` to a draft release named after the version; publish the draft
   on GitHub and the `/download` link and auto-update both go live.

Without a token, `npm run dist:win` and drag the three files from `dist/`
onto a new release by hand — the asset must be named exactly `ImagineOne-Setup.exe`.
