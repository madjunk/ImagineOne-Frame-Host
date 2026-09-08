import { app, BrowserWindow, WebContentsView, shell, ipcMain, nativeTheme, dialog, Tray, Menu, nativeImage, screen } from 'electron'
import { join } from 'path'
import { autoUpdater } from 'electron-updater'
import { getConfig, setConfig, applyPackagedDefaults } from './config'
import { APP_PARTITION, appSession, checkDeployment, fetchTenant, fetchMe, logout } from './api'
import { buildMenu } from './menu'
import { resolveBranding } from './branding'
import { getCredentials, saveCredentials, clearCredentials } from './credentials'
import { COMPACT_WINDOW, DEFAULT_BRANDING, TITLE_BAR_HEIGHT, type FrameState, type FrameConfig, type FrameBranding } from '../shared/types'

// ---------------------------------------------------------------------------
// Single instance — a second launch just focuses the existing window.
// ---------------------------------------------------------------------------
if (!app.requestSingleInstanceLock()) {
  app.quit()
}

let win: BrowserWindow | null = null
let appView: WebContentsView | null = null
let tray: Tray | null = null
let quitting = false

const state: FrameState = {
  phase: 'starting',
  sidebar: null,
  config: getConfig(),
  branding: DEFAULT_BRANDING,
  user: null,
  errorMessage: null,
  orgError: null,
  orgBusy: false,
  online: true,
  canGoBack: false,
  canGoForward: false,
  appVersion: app.getVersion(),
  platform: process.platform
}

function pushState(patch: Partial<FrameState> = {}): void {
  Object.assign(state, patch)
  win?.webContents.send('frame:state', state)
  Menu.setApplicationMenu(buildMenu(state, actions))
}

// ---------------------------------------------------------------------------
// Navigation scope: only this install's own host may load inside the frame.
// ---------------------------------------------------------------------------
function isAllowedUrl(url: string): boolean {
  try {
    const target = new URL(url)
    const base = new URL(state.config.baseUrl)
    return target.host === base.host && (target.protocol === 'https:' || target.protocol === 'http:')
  } catch {
    return false
  }
}

/**
 * Identity providers the app signs in through. These may load inside the
 * frame (as a child popup or a top-level redirect) because the OAuth flow has
 * to end back in the frame's own session, not the system browser.
 */
const AUTH_HOST_PATTERNS = [
  /(^|\.)accounts\.google\.com$/,
  /(^|\.)accounts\.youtube\.com$/,
  /^oauth2\.googleapis\.com$/,
  /(^|\.)login\.microsoftonline\.com$/,
  /(^|\.)login\.live\.com$/,
  /(^|\.)login\.microsoft\.com$/,
  /(^|\.)account\.microsoft\.com$/,
  /(^|\.)auth0\.com$/,
  /(^|\.)okta\.com$/
]

function isAuthProviderUrl(url: string): boolean {
  try {
    const { protocol, hostname } = new URL(url)
    return protocol === 'https:' && AUTH_HOST_PATTERNS.some((re) => re.test(hostname))
  } catch {
    return false
  }
}

let authPopup: BrowserWindow | null = null

/**
 * Sign-in popup for an identity provider. Shares the app's session partition,
 * so when the provider redirects back to the app's callback the session cookie
 * lands in the frame; at that point the popup closes and the main view follows
 * the callback URL (the cookie is already set, so the app comes up signed in).
 */
function openAuthPopup(url: string): void {
  if (authPopup && !authPopup.isDestroyed()) {
    authPopup.focus()
    void authPopup.loadURL(url)
    return
  }
  const popup = new BrowserWindow({
    parent: win ?? undefined,
    modal: false,
    width: 520,
    height: 700,
    resizable: false,
    minimizable: false,
    maximizable: false,
    autoHideMenuBar: true,
    title: 'Sign in',
    backgroundColor: '#ffffff',
    webPreferences: { partition: APP_PARTITION, contextIsolation: true, nodeIntegration: false, sandbox: true }
  })
  authPopup = popup
  const pwc = popup.webContents
  pwc.setUserAgent(appView?.webContents.getUserAgent() ?? pwc.getUserAgent())

  let handedBack = false
  const finish = (finalUrl: string) => {
    if (handedBack) return
    handedBack = true
    // Let the callback's own redirect chain settle in the popup, then move
    // the resulting page into the frame and close the popup.
    setTimeout(() => {
      void appView?.webContents.loadURL(finalUrl)
      if (!popup.isDestroyed()) popup.close()
    }, 150)
  }

  pwc.on('will-navigate', (event, target) => {
    if (isAllowedUrl(target) || isAuthProviderUrl(target)) return
    event.preventDefault()
    openExternally(target)
  })
  pwc.setWindowOpenHandler(({ url: target }) => {
    if (isAuthProviderUrl(target)) void pwc.loadURL(target)
    else openExternally(target)
    return { action: 'deny' }
  })
  // Back on our domain = the provider is done. did-navigate fires after the
  // callback's redirect chain, so the session cookie is already set.
  pwc.on('did-navigate', (_e, target) => {
    if (isAllowedUrl(target)) finish(target)
  })
  popup.on('closed', () => {
    authPopup = null
    void refreshUser().then(refreshBranding)
  })
  void popup.loadURL(url)
}

function openExternally(url: string): void {
  // mailto:, tel:, other domains — hand to the OS default handler.
  if (/^(https?|mailto|tel):/i.test(url)) void shell.openExternal(url)
}

// ---------------------------------------------------------------------------
// Window + app view
// ---------------------------------------------------------------------------
function layoutAppView(): void {
  if (!win || !appView) return
  const { width, height } = win.getContentBounds()
  appView.setBounds({ x: 0, y: TITLE_BAR_HEIGHT, width, height: Math.max(0, height - TITLE_BAR_HEIGHT) })
}

function showAppView(visible: boolean): void {
  if (!win || !appView) return
  appView.setVisible(visible)
  if (visible) layoutAppView()
}

type WindowMode = 'compact' | 'full'
let windowMode: WindowMode = 'compact'
let fullBounds: Electron.Rectangle | null = null

/**
 * Compact = the small sign-in window (fixed size, centred). Full = the app.
 * Remembers the full-size bounds so sign-out/sign-in round-trips restore them.
 */
function setWindowMode(mode: WindowMode): void {
  if (!win || mode === windowMode) return
  if (mode === 'compact') {
    if (!win.isMaximized() && !win.isFullScreen()) fullBounds = win.getBounds()
    if (win.isMaximized()) win.unmaximize()
    win.setResizable(false)
    win.setMinimumSize(COMPACT_WINDOW.width, COMPACT_WINDOW.height)
    win.setSize(COMPACT_WINDOW.width, COMPACT_WINDOW.height, true)
    win.center()
  } else {
    win.setMinimumSize(800, 500)
    win.setResizable(true)
    if (fullBounds && boundsOnScreen(fullBounds)) win.setBounds(fullBounds, true)
    else {
      // Fit the monitor the window is on: 92% of its work area (never larger
      // than the screen, so the bottom of the page is never pushed off-screen),
      // capped at a comfortable desktop size, and centred.
      const { workArea } = screen.getDisplayMatching(win.getBounds())
      const width = Math.min(1500, Math.round(workArea.width * 0.92))
      const height = Math.min(1000, Math.round(workArea.height * 0.92))
      win.setBounds(
        {
          width,
          height,
          x: workArea.x + Math.round((workArea.width - width) / 2),
          y: workArea.y + Math.round((workArea.height - height) / 2)
        },
        true
      )
    }
  }
  windowMode = mode
  layoutAppView()
}

/** True when the rectangle lies fully inside some display's work area. */
function boundsOnScreen(b: Electron.Rectangle): boolean {
  return screen.getAllDisplays().some(({ workArea: w }) =>
    b.x >= w.x && b.y >= w.y && b.x + b.width <= w.x + w.width && b.y + b.height <= w.y + w.height
  )
}

function createAppView(): WebContentsView {
  const view = new WebContentsView({
    webPreferences: {
      partition: APP_PARTITION,
      preload: join(__dirname, '../preload/app.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      spellcheck: true
    }
  })
  const wc = view.webContents

  // Identify the frame to the server: the User-Agent gains an
  // "ImagineOneFrame/<version>" token and every request carries
  // X-ImagineOne-Frame. The server uses either to skip the forced-PWA gate
  // (useStandaloneGuard) for frame traffic only, leaving PWA_REQUIRED intact
  // for ordinary browsers.
  // "Electron/x.y" is dropped: Google refuses OAuth sign-in from anything it
  // classifies as an embedded browser, and it keys on that token.
  const baseUa = wc.getUserAgent().replace(/\sElectron\/[\d.]+/, '').replace(/\s+/g, ' ')
  wc.setUserAgent(`${baseUa} ImagineOneFrame/${app.getVersion()}`)

  wc.on('will-navigate', (event, url) => {
    if (isAllowedUrl(url)) return
    event.preventDefault()
    // The app signs in with Google/Microsoft via a full-page redirect
    // (window.location.href = authUrl). Run that in a popup attached to the
    // frame instead, so the frame itself never leaves the app.
    if (isAuthProviderUrl(url)) openAuthPopup(url)
    else openExternally(url)
  })
  wc.setWindowOpenHandler(({ url }) => {
    if (isAuthProviderUrl(url)) {
      openAuthPopup(url)
      return { action: 'deny' }
    }
    if (isAllowedUrl(url)) {
      // target=_blank links to our own domain stay inside the frame.
      void wc.loadURL(url)
    } else {
      openExternally(url)
    }
    return { action: 'deny' }
  })

  const syncNav = () =>
    pushState({
      canGoBack: wc.navigationHistory.canGoBack(),
      canGoForward: wc.navigationHistory.canGoForward()
    })
  wc.on('did-start-navigation', (details) => {
    if (details.isMainFrame && !details.isSameDocument && state.sidebar) pushState({ sidebar: null })
  })
  wc.on('did-navigate', () => {
    syncNav()
    void refreshUser().then(refreshBranding)
  })
  wc.on('did-navigate-in-page', syncNav)
  wc.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    if (isMainFrame && code !== -3 /* ABORTED */) {
      pushState({ phase: 'config-error', errorMessage: `Could not load ${url}: ${desc} (${code})` })
      showAppView(false)
    }
  })
  wc.on('page-title-updated', (e, title) => {
    e.preventDefault()
    win?.setTitle(title || state.branding.name)
  })

  return view
}

function createWindow(): void {
  win = new BrowserWindow({
    width: COMPACT_WINDOW.width,
    height: COMPACT_WINDOW.height,
    resizable: false,
    center: true,
    show: false,
    title: 'ImagineOne',
    icon: join(app.isPackaged ? process.resourcesPath : join(__dirname, '../../build'), 'icon.png'),
    backgroundColor: nativeTheme.shouldUseDarkColors ? '#111418' : '#ffffff',
    // Frameless with native window controls: the renderer draws the bar.
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'hidden',
    titleBarOverlay:
      process.platform === 'darwin'
        ? undefined
        : { color: DEFAULT_BRANDING.barBackground, symbolColor: DEFAULT_BRANDING.barForeground, height: TITLE_BAR_HEIGHT },
    trafficLightPosition: { x: 14, y: 12 },
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true
    }
  })

  appView = createAppView()
  win.contentView.addChildView(appView)
  appView.setVisible(false)

  win.on('resize', layoutAppView)
  win.on('maximize', layoutAppView)
  win.on('unmaximize', layoutAppView)
  win.on('enter-full-screen', layoutAppView)
  win.on('leave-full-screen', layoutAppView)
  win.on('ready-to-show', () => win?.show())
  win.on('close', (e) => {
    // Minimize to tray instead of quitting (Windows/Linux). Cmd+Q / Quit menu sets `quitting`.
    if (!quitting && process.platform !== 'darwin') {
      e.preventDefault()
      win?.hide()
    }
  })
  win.on('closed', () => {
    win = null
    appView = null
  })

  // Renderer chrome must never navigate away or open windows.
  win.webContents.setWindowOpenHandler(({ url }) => {
    openExternally(url)
    return { action: 'deny' }
  })

  if (process.env['ELECTRON_RENDERER_URL']) {
    void win.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    void win.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

// ---------------------------------------------------------------------------
// Boot sequence per frame-host-api-reference.md
//   1. GET /api/public/deployment  — reachability
//   2. GET /api/public/tenant/:slug — branding + install lock (if slug configured)
//   3. show the app view; role from /api/auth/me drives the native menu
// ---------------------------------------------------------------------------
async function boot(): Promise<void> {
  pushState({ phase: 'starting', errorMessage: null, config: getConfig() })
  showAppView(false)
  const { baseUrl, tenantSlug } = state.config

  try {
    await checkDeployment(baseUrl)
  } catch (err) {
    pushState({ phase: 'config-error', errorMessage: `Cannot reach ${baseUrl}. ${(err as Error).message}` })
    return
  }

  if (tenantSlug) {
    let tenant
    try {
      tenant = await fetchTenant(baseUrl, tenantSlug)
    } catch (err) {
      pushState({ phase: 'config-error', errorMessage: (err as Error).message })
      return
    }
    if (!tenant || tenant.slug !== tenantSlug) {
      pushState({
        phase: 'config-error',
        errorMessage: `This install is configured for company "${tenantSlug}", but ${baseUrl} does not serve that company.`
      })
      return
    }
  }

  // Already signed in from a previous run? Straight to the app.
  await refreshUser()
  if (state.user) {
    await enterApp()
    return
  }

  if (tenantSlug) await enterLogin(tenantSlug)
  else await enterOrgStep()
}

/** Compact window, frame-drawn organisation step. */
async function enterOrgStep(): Promise<void> {
  setWindowMode('compact')
  showAppView(false)
  pushState({ phase: 'org', orgError: null, orgBusy: false })
  await refreshBranding()
}

/** Compact window, the app's own /t/:slug sign-in page. */
async function enterLogin(slug: string): Promise<void> {
  setWindowMode('compact')
  showAppView(false)
  pushState({ phase: 'login', orgError: null, orgBusy: false })
  await refreshBranding()
  await appView?.webContents.loadURL(`${state.config.baseUrl}/t/${encodeURIComponent(slug)}`)
  if (state.phase === 'login') showAppView(true)
}

/** Full window, signed in. */
async function enterApp(): Promise<void> {
  setWindowMode('full')
  pushState({ phase: 'ready', orgError: null, orgBusy: false })
  await refreshBranding()
  const url = appView?.webContents.getURL() ?? ''
  const onLoginPage = /\/t\/[^/]+\/?$/.test(new URL(url || state.config.baseUrl).pathname) || !url
  if (onLoginPage) await appView?.webContents.loadURL(state.config.baseUrl + '/')
  showAppView(true)
}

/**
 * Re-derive title bar branding from whatever applies now (signed-in tenant
 * settings → /t/:slug tenant → public site settings) and repaint the native
 * caption buttons to match.
 */
async function refreshBranding(): Promise<void> {
  const branding = await resolveBranding({
    baseUrl: state.config.baseUrl,
    configuredSlug: state.config.tenantSlug,
    // On the organisation step no company is chosen yet, so ignore whatever
    // /t/:slug page the app view still shows and brand as the platform.
    currentUrl: state.phase === 'org' ? '' : (appView?.webContents.getURL() ?? ''),
    signedIn: !!state.user
  })
  if (JSON.stringify(branding) === JSON.stringify(state.branding)) return
  console.log(
    `[frame] branding ← ${branding.source}: "${branding.name}", logo=${branding.logoUrl ? (branding.logoUrl.startsWith('data:') ? 'data-url' : branding.logoUrl) : 'none'}, bar=${branding.barBackground}, slug=${state.config.tenantSlug || '-'}, signedIn=${!!state.user}`
  )
  applyBranding(branding)
}

/** rgb()/rgba() → #rrggbb (titleBarOverlay wants hex); hex passes through. */
function cssToHex(color: string): string {
  const m = color.match(/rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/i)
  if (!m) return color
  return '#' + [m[1], m[2], m[3]].map((n) => Number(n).toString(16).padStart(2, '0')).join('')
}

function applyBranding(branding: FrameBranding): void {
  pushState({ branding })
  if (win && process.platform !== 'darwin') {
    win.setTitleBarOverlay({ color: branding.barBackground, symbolColor: branding.barForeground, height: TITLE_BAR_HEIGHT })
  }
  if (win && !appView?.webContents.getTitle()) win.setTitle(branding.name)
}

async function refreshUser(): Promise<void> {
  const user = await fetchMe(state.config.baseUrl)
  if (JSON.stringify(user) !== JSON.stringify(state.user)) pushState({ user })
  // Signed in while on the compact login page → grow into the app.
  if (user && state.phase === 'login') await enterApp()
  // Session gone while in the app (expired / signed out in-page) → back to login.
  if (!user && state.phase === 'ready') {
    if (state.config.tenantSlug) await enterLogin(state.config.tenantSlug)
    else await enterOrgStep()
  }
}

// ---------------------------------------------------------------------------
// Actions shared by the native menu, tray and renderer title bar.
// ---------------------------------------------------------------------------
export interface FrameActions {
  continueWithOrg(slug?: unknown): Promise<void>
  changeOrg(): Promise<void>
  home(): void
  reload(): void
  back(): void
  forward(): void
  navigate(path: string): void
  signOut(): Promise<void>
  openSettings(): void
  saveSettings(next: Partial<FrameConfig>): Promise<void>
  checkForUpdates(): void
  about(): void
  quit(): void
}

/** Frame settings are admin-only; changing organisation is also allowed before sign-in. */
function canChangeSetup(): boolean {
  return !!state.user && (state.user.role === 'admin' || state.user.role === 'super_admin')
}

const actions: FrameActions = {
  home: () => void appView?.webContents.loadURL(state.config.baseUrl + '/'),
  reload: () => appView?.webContents.reload(),
  back: () => appView?.webContents.navigationHistory.goBack(),
  forward: () => appView?.webContents.navigationHistory.goForward(),
  navigate: (path) => {
    const url = path.startsWith('http') ? path : state.config.baseUrl + path
    if (isAllowedUrl(url)) void appView?.webContents.loadURL(url)
    else openExternally(url)
  },
  signOut: async () => {
    await logout(state.config.baseUrl)
    pushState({ user: null })
    if (state.config.tenantSlug) await enterLogin(state.config.tenantSlug)
    else await enterOrgStep()
  },
  continueWithOrg: async (slugArg) => {
    const slug = String(slugArg ?? '').trim().toLowerCase()
    if (!slug) {
      pushState({ orgError: 'Enter your organisation identifier.' })
      return
    }
    pushState({ orgBusy: true, orgError: null })
    try {
      const tenant = await fetchTenant(state.config.baseUrl, slug)
      if (!tenant) {
        pushState({ orgBusy: false, orgError: `No organisation "${slug}" was found. Check the identifier from your administrator.` })
        return
      }
    } catch (err) {
      pushState({ orgBusy: false, orgError: `Could not verify the organisation: ${(err as Error).message}` })
      return
    }
    setConfig({ tenantSlug: slug })
    pushState({ config: getConfig() })
    await enterLogin(slug)
  },
  changeOrg: async () => {
    if (state.user && !canChangeSetup()) return
    setConfig({ tenantSlug: '' })
    pushState({ config: getConfig() })
    await enterOrgStep()
  },
  openSettings: () => {
    if (!canChangeSetup()) return
    showAppView(false)
    win?.webContents.send('frame:open-settings')
  },
  saveSettings: async (next) => {
    if (!canChangeSetup()) return
    setConfig(next)
    await boot()
  },
  checkForUpdates: () => {
    if (!app.isPackaged) {
      void dialog.showMessageBox({ message: 'Update checks only run in the packaged app.' })
      return
    }
    void autoUpdater.checkForUpdatesAndNotify()
  },
  about: () => {
    const name = state.branding.name
    void dialog.showMessageBox({
      type: 'info',
      title: `About ${name}`,
      message: name,
      detail: `Application Frame Host ${app.getVersion()}\nElectron ${process.versions.electron} · Chromium ${process.versions.chrome}\n\n${state.config.baseUrl}`
    })
  },
  quit: () => {
    quitting = true
    app.quit()
  }
}

// ---------------------------------------------------------------------------
// IPC from the renderer title bar
// ---------------------------------------------------------------------------
function registerIpc(): void {
  ipcMain.handle('frame:get-state', () => state)
  ipcMain.on('frame:action', (_e, name: keyof FrameActions, arg?: unknown) => {
    const fn = actions[name] as (a?: unknown) => unknown
    if (typeof fn === 'function') void fn(arg)
  })
  ipcMain.on('frame:sidebar', (e, metrics: FrameState['sidebar']) => {
    if (e.sender !== appView?.webContents) return
    const m = metrics && typeof metrics.width === 'number' ? metrics : null
    if (JSON.stringify(m) !== JSON.stringify(state.sidebar)) pushState({ sidebar: m })
  })
  // "Remember me" bridge for the hosted app's sign-in page. Only the app view
  // (our own origin) may use it; auth popups and the title bar cannot.
  const fromAppView = (e: Electron.IpcMainInvokeEvent) => e.sender === appView?.webContents
  ipcMain.handle('frame:credentials:get', (e, slug: string) => (fromAppView(e) ? getCredentials(slug) : null))
  ipcMain.handle('frame:credentials:save', (e, slug: string, creds: { email: string; password: string }) => {
    if (!fromAppView(e) || !slug || !creds?.email || !creds?.password) return
    saveCredentials(slug, { email: creds.email, password: creds.password })
  })
  ipcMain.handle('frame:credentials:clear', (e, slug: string) => {
    if (fromAppView(e) && slug) clearCredentials(slug)
  })
  ipcMain.on('frame:open-menu', (_e, x: number, y: number) => {
    const menu = Menu.getApplicationMenu()
    if (win && menu) menu.popup({ window: win, x: Math.round(x), y: Math.round(y) })
  })
  ipcMain.on('frame:settings-closed', () => {
    if (state.phase === 'ready' || state.phase === 'login') showAppView(true)
  })
  ipcMain.on('frame:online', (_e, online: boolean) => pushState({ online }))
  // The title bar tells us the colour of its right side; the OS-drawn caption
  // buttons (Windows) are painted to match.
  ipcMain.on('frame:bar-colors', (e, background: string, foreground: string) => {
    if (e.sender !== win?.webContents || process.platform === 'darwin') return
    const color = cssToHex(String(background))
    const symbolColor = cssToHex(String(foreground))
    if (/^#[0-9a-f]{6}$/i.test(color) && /^#[0-9a-f]{6}$/i.test(symbolColor)) {
      win.setTitleBarOverlay({ color, symbolColor, height: TITLE_BAR_HEIGHT })
    }
  })
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------
function createTray(): void {
  const iconPath = join(app.isPackaged ? process.resourcesPath : join(__dirname, '../../resources'), 'tray.png')
  const icon = nativeImage.createFromPath(iconPath)
  if (icon.isEmpty()) return
  tray = new Tray(process.platform === 'darwin' ? icon.resize({ width: 18, height: 18 }) : icon)
  tray.setToolTip('ImagineOne')
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: 'Open ImagineOne', click: () => win?.show() },
      { label: 'Home', click: actions.home },
      { type: 'separator' },
      { label: 'Check for updates', click: actions.checkForUpdates },
      { type: 'separator' },
      { label: 'Quit', click: actions.quit }
    ])
  )
  tray.on('click', () => (win?.isVisible() ? win.focus() : win?.show()))
}

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------
app.on('second-instance', () => {
  if (win) {
    win.show()
    if (win.isMinimized()) win.restore()
    win.focus()
  }
})

app.on('before-quit', () => {
  quitting = true
})

app.on('activate', () => {
  if (win) win.show()
  else createWindow()
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

app.whenReady().then(() => {
  app.setAppUserModelId('it.imagineone.frame')
  applyPackagedDefaults(join(app.isPackaged ? process.resourcesPath : process.cwd(), 'frame-config.json'))
  state.config = getConfig()

  // Every request from the app partition (page loads, XHR/fetch, and the
  // frame's own API calls) carries the frame header + UA token.
  const frameUa = `ImagineOneFrame/${app.getVersion()}`
  appSession().webRequest.onBeforeSendHeaders((details, callback) => {
    const requestHeaders: Record<string, string> = { ...details.requestHeaders, 'X-ImagineOne-Frame': app.getVersion() }
    const ua = requestHeaders['User-Agent']
    if (ua && !ua.includes('ImagineOneFrame/')) requestHeaders['User-Agent'] = `${ua} ${frameUa}`
    callback({ requestHeaders })
  })

  // Downloads: let the OS save dialog handle them (default behaviour) but keep
  // them out of the frame's navigation.
  appSession().on('will-download', (_e, item) => {
    item.once('done', (_ev, s) => {
      if (s === 'completed') shell.showItemInFolder(item.getSavePath())
    })
  })

  autoUpdater.autoDownload = true
  autoUpdater.on('update-downloaded', () => {
    void dialog
      .showMessageBox({ type: 'info', buttons: ['Restart now', 'Later'], message: 'An update is ready. Restart to install?' })
      .then((r) => r.response === 0 && autoUpdater.quitAndInstall())
  })

  registerIpc()
  createWindow()
  createTray()
  Menu.setApplicationMenu(buildMenu(state, actions))
  win?.webContents.once('did-finish-load', () => void boot())
  if (app.isPackaged) setTimeout(() => void autoUpdater.checkForUpdatesAndNotify().catch(() => {}), 10_000)
})
