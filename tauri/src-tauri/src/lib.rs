//! Imagine One IT — desktop app (Tauri build of imagineone-frame-host).
//!
//! The same design as the Electron app (../../src/main): one window with our own glass title bar
//! (webview "chrome": src/renderer, built to tauri/ui-dist) and, below it, the ImagineOne web app
//! (webview "app") loaded from the configured server. Navigation in the app is locked to that
//! server; Google / Microsoft sign-in runs in a small popup window that shares the app's cookies;
//! everything else opens in the default browser. The window is compact for the organisation step
//! and the sign-in page, and full size (remembered) once signed in. Closing it keeps the app in the
//! tray.
//!
//! Bridges:
//! - window.frameHost (title bar): src/renderer/src/tauri-bridge.ts → the frame_* chrome commands.
//! - window.imagineOneFrame (web app, same API as the Electron preload): APP_JS below → saved
//!   sign-in ("Remember me"), who is signed in, the sidebar size and colours for the title bar.
//!   Only the configured server may call those: its capability is added at run time for that
//!   address only, and every command checks the page again.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    ipc::CapabilityBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    path::BaseDirectory,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    utils::config::{Color, WindowEffectsConfig},
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, WebviewBuilder},
    window::{Effect, WindowBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

const TITLE_BAR: f64 = 48.0;
const COMPACT: (f64, f64) = (480.0, 700.0);
const FULL_MIN: (f64, f64) = (800.0, 500.0);
const DEFAULT_BASE: &str = "https://app.imagineone.it";
/// The web app's default sidebar colour (hsl(199,100%,22%)).
const APP_DEFAULT_SIDEBAR: &str = "#004d70";

// ---------------------------------------------------------------------------------------------
// Configuration: the same file as the Electron app, so an install keeps its server and company.
//   standard build:  %APPDATA%\<product name>\frame-config.json  (Imagine One IT)
//   customer build:  %APPDATA%\ImagineOne\<slug>\frame-config.json
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
struct FrameConfig {
    base_url: String,
    tenant_slug: String,
}

#[derive(Default, Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Default, Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    tenant_slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seeded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seeded_stamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bounds: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximized: Option<bool>,
    #[serde(flatten)]
    other: Map<String, Value>,
}

/// resources/frame-config.json: { baseUrl, tenantSlug } (a customer build locks the slug).
#[derive(Default, Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Packaged {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    tenant_slug: Option<String>,
}

struct Config {
    dir: PathBuf,
    packaged: Option<(String, Packaged)>,
    stored: Stored,
    /// Desktop.ps1 Dev -Server …: a server for this debug run only (not saved).
    dev_server: Option<String>,
}

fn clean_slug(s: &str) -> String {
    s.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(60).collect()
}

impl Config {
    fn packaged_slug(packaged: &Option<(String, Packaged)>) -> String {
        packaged.as_ref().and_then(|(_, p)| p.tenant_slug.as_deref()).map(clean_slug).unwrap_or_default()
    }
    fn file(&self) -> PathBuf {
        self.dir.join("frame-config.json")
    }
    fn get(&self) -> FrameConfig {
        let base = self.dev_server.as_deref().unwrap_or(&self.stored.base_url);
        FrameConfig { base_url: normalize_base_url(base), tenant_slug: clean_slug(&self.stored.tenant_slug) }
    }
    fn set(&mut self, base_url: Option<&str>, tenant_slug: Option<&str>) {
        if let Some(b) = base_url {
            self.stored.base_url = normalize_base_url(b);
        }
        if let Some(t) = tenant_slug {
            self.stored.tenant_slug = clean_slug(t);
        }
        self.save();
    }
    fn save(&self) {
        let _ = fs::create_dir_all(&self.dir);
        if let Ok(text) = serde_json::to_string_pretty(&self.stored) {
            let _ = fs::write(self.file(), text);
        }
    }
    /// Apply the packaged frame-config.json on first run, and again whenever its contents change
    /// (a corrected installer really updates an install that already ran). Same as Electron.
    fn apply_packaged_defaults(&mut self) {
        let Some((raw, p)) = self.packaged.clone() else { return };
        let stamp = sha1_smol::Sha1::from(raw.as_bytes()).digest().to_string();
        if self.stored.seeded == Some(true) && self.stored.seeded_stamp.as_deref() == Some(stamp.as_str()) {
            return;
        }
        if let Some(b) = &p.base_url {
            self.stored.base_url = normalize_base_url(b);
        }
        if let Some(t) = &p.tenant_slug {
            self.stored.tenant_slug = clean_slug(t);
        }
        self.stored.seeded = Some(true);
        self.stored.seeded_stamp = Some(stamp);
        self.save();
    }
}

/// "app.imagineone.it", a full page URL, trailing slashes… → https://app.imagineone.it
fn normalize_base_url(input: &str) -> String {
    let mut s = input.trim().to_string();
    if s.is_empty() {
        return DEFAULT_BASE.into();
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        s = format!("https://{s}");
    }
    match Url::parse(&s) {
        Ok(u) if u.host_str().is_some() => u.origin().ascii_serialization(),
        _ => DEFAULT_BASE.into(),
    }
}

/// host[:port], as Electron's URL.host.
fn host_of(u: &Url) -> Option<String> {
    let h = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{h}:{p}"),
        None => h.to_string(),
    })
}

fn is_allowed(base: &str, url: &Url) -> bool {
    let Ok(b) = Url::parse(base) else { return false };
    matches!(url.scheme(), "http" | "https") && host_of(url).is_some() && host_of(url) == host_of(&b)
}

/// Identity providers the app signs in through: they open in the sign-in popup (sharing the app's
/// cookies) so the flow ends back in the app, not in the browser.
fn is_auth_provider(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let h = url.host_str().unwrap_or("").to_ascii_lowercase();
    let ends = |d: &str| h == d || h.ends_with(&format!(".{d}"));
    ends("accounts.google.com")
        || ends("accounts.youtube.com")
        || h == "oauth2.googleapis.com"
        || ends("login.microsoftonline.com")
        || ends("login.live.com")
        || ends("login.microsoft.com")
        || ends("account.microsoft.com")
        || ends("auth0.com")
        || ends("okta.com")
}

/// The slug of a /t/:slug sign-in page.
fn slug_from_url(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let mut parts = u.path_segments()?;
    if parts.next()? != "t" {
        return None;
    }
    let s = parts.next()?;
    (!s.is_empty()).then(|| clean_slug(&url::form_urlencoded::parse(s.as_bytes()).map(|(k, _)| k.to_string()).next().unwrap_or_default()))
}

fn is_login_path(url: &Url) -> bool {
    let segs: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    segs.len() == 2 && segs[0] == "t"
}

// ---------------------------------------------------------------------------------------------
// Title bar branding (same rules as src/main/branding.ts)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
struct Branding {
    source: &'static str,
    name: String,
    logo_url: Option<String>,
    bar_background: String,
    bar_foreground: String,
    primary_color: String,
    bar_explicit: bool,
    logo_background: Option<String>,
}

fn default_branding(name: &str) -> Branding {
    Branding {
        source: "default",
        name: name.into(),
        logo_url: None,
        bar_background: "#f5f6f8".into(),
        bar_foreground: "#1f2328".into(),
        primary_color: "#2563eb".into(),
        bar_explicit: false,
        logo_background: None,
    }
}

fn hex(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    let h = s.strip_prefix('#')?;
    if !(h.len() == 3 || h.len() == 6) || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(if h.len() == 3 { format!("#{}", h.chars().flat_map(|c| [c, c]).collect::<String>()) } else { format!("#{h}") }.to_lowercase())
}

fn contrast_foreground(bg: &str) -> String {
    let h = bg.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    let lin = |c: f64| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let l = 0.2126 * lin(c(0)) + 0.7152 * lin(c(2)) + 0.0722 * lin(c(4));
    if l > 0.5 { "#1f2328" } else { "#ffffff" }.into()
}

/// A logo the title bar may show: https/http on the server, or an image data URL.
fn absolutize(base: &str, url: Option<&Value>) -> Option<String> {
    let s = url?.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    if s.to_ascii_lowercase().starts_with("data:image/") {
        return Some(s.to_string());
    }
    let u = Url::parse(&format!("{base}/")).ok()?.join(s).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.to_string())
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn from_tenant(base: &str, t: &Value) -> Branding {
    let def = default_branding("");
    let bar = hex(t.get("primaryColor")).unwrap_or(def.bar_background);
    Branding {
        source: "tenant",
        name: text(t, "appTitle").or_else(|| text(t, "name")).unwrap_or_default(),
        logo_url: absolutize(base, t.get("logoUrl")),
        bar_foreground: contrast_foreground(&bar),
        primary_color: hex(t.get("accentColor")).unwrap_or_else(|| bar.clone()),
        bar_background: bar,
        bar_explicit: false,
        logo_background: None,
    }
}

fn from_settings(base: &str, s: &Value, product: &str) -> Option<Branding> {
    let name = text(s, "company-name").or_else(|| text(s, "app-title"));
    let logo = s.get("company-logo-url");
    let title_bg = hex(s.get("theme-sidebar-color")).unwrap_or_else(|| APP_DEFAULT_SIDEBAR.into());
    let primary = hex(s.get("theme-primary-color"));
    let text_c = hex(s.get("theme-text-color"));
    let explicit_bar = hex(s.get("theme-titlebar-background-color"));
    let explicit_text = hex(s.get("theme-titlebar-text-color"));
    let has_logo = logo.and_then(|l| l.as_str()).is_some_and(|l| !l.is_empty());
    // title_bg always has a value (the app's default sidebar), as in branding.ts
    if name.is_none() && !has_logo && primary.is_none() && explicit_bar.is_none() && title_bg.is_empty() {
        return None;
    }
    let (bar, fg) = match &explicit_bar {
        Some(b) => (b.clone(), explicit_text.unwrap_or_else(|| contrast_foreground(b))),
        None => (title_bg.clone(), text_c.unwrap_or_else(|| contrast_foreground(&title_bg))),
    };
    Some(Branding {
        source: "settings",
        name: name.unwrap_or_else(|| product.to_string()),
        logo_url: absolutize(base, logo),
        primary_color: primary.unwrap_or_else(|| bar.clone()),
        bar_background: bar,
        bar_foreground: fg,
        bar_explicit: explicit_bar.is_some(),
        logo_background: hex(s.get("theme-logo-background-color")),
    })
}

fn from_site(base: &str, s: &Value, product: &str) -> Branding {
    let def = default_branding(product);
    let name = text(s, "siteName").or_else(|| text(s, "companyName")).or_else(|| text(s, "appTitle")).unwrap_or_else(|| product.into());
    let primary = hex(s.get("primaryColor")).or_else(|| hex(s.get("themePrimaryColor")));
    let bar = primary.clone().unwrap_or(def.bar_background);
    Branding {
        source: "site",
        name,
        logo_url: absolutize(base, s.get("logoUrl")),
        bar_foreground: contrast_foreground(&bar),
        primary_color: primary.unwrap_or(def.primary_color),
        bar_background: bar,
        bar_explicit: false,
        logo_background: None,
    }
}

// ---------------------------------------------------------------------------------------------
// State shown by the title bar (same shape as src/shared/types.ts FrameState)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
struct Sidebar {
    left: f64,
    width: f64,
    color: String,
    page_color: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameState {
    phase: &'static str, // starting | config-error | org | login | ready
    sidebar: Option<Sidebar>,
    config: FrameConfig,
    branding: Branding,
    user: Option<Value>,
    error_message: Option<String>,
    org_error: Option<String>,
    org_busy: bool,
    online: bool,
    can_go_back: bool,
    can_go_forward: bool,
    maximized: bool,
    app_version: String,
    platform: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Compact,
    Full,
}

struct Frame {
    cfg: Config,
    s: FrameState,
    product: String,
    mode: Mode,
    allowed: Vec<String>,
    /// Booting: waiting for the page to say whether someone is signed in.
    awaiting_user: bool,
    boot_id: u64,
    /// Settings of the signed-in tenant, as the page last read them (/api/settings).
    settings: Option<Value>,
    cache: HashMap<String, (Instant, Option<Value>)>,
    zoom: f64,
    updating: bool,
    quitting: bool,
    normal: Option<Bounds>,
}
type Shared = Arc<Mutex<Frame>>;

impl Frame {
    fn base(&self) -> String {
        self.cfg.get().base_url
    }
    fn is_server(&self, url: &Url) -> bool {
        is_allowed(&self.base(), url)
    }
    fn is_admin(&self) -> bool {
        self.s.user.as_ref().and_then(|u| u.get("role")).and_then(|r| r.as_str()).is_some_and(|r| r == "admin" || r == "super_admin")
    }
}

fn shared(app: &AppHandle) -> Shared {
    app.state::<Shared>().inner().clone()
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        _ => "linux",
    }
}

/// Our own pages: tauri://localhost (macOS/Linux) or http://tauri.localhost (Windows).
fn is_local(url: &Url) -> bool {
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
}

fn main_window(app: &AppHandle) -> Option<Window> {
    app.get_window("main")
}
fn chrome(app: &AppHandle) -> Option<Webview> {
    app.get_webview("chrome")
}
fn app_view(app: &AppHandle) -> Option<Webview> {
    app.get_webview("app")
}

/// Change the state and show it in the title bar.
fn push(app: &AppHandle, f: impl FnOnce(&mut FrameState)) {
    let json = {
        let st = shared(app);
        let mut c = st.lock().unwrap();
        f(&mut c.s);
        serde_json::to_string(&c.s).unwrap_or_default()
    };
    if let Some(w) = chrome(app) {
        let _ = w.eval(format!("window.__frameState && window.__frameState({json})"));
    }
}

fn state_of(app: &AppHandle) -> FrameState {
    shared(app).lock().unwrap().s.clone()
}

// ---------------------------------------------------------------------------------------------
// window.imagineOneFrame for the web app (same API as src/preload/app.ts), plus the sidebar
// watcher, who is signed in, and the keyboard shortcuts the Electron menu had.
// ---------------------------------------------------------------------------------------------

const APP_JS: &str = r#"(function () {
  if (window.top !== window || window.imagineOneFrame || !/^https?:$/.test(location.protocol)) return;
  var call = function (cmd, args) {
    try { return window.__TAURI_INTERNALS__.invoke(cmd, args || {}); } catch (e) { return Promise.reject(e); }
  };
  var origFetch = window.fetch;

  // Who is signed in (and the tenant's theme): read with the page's own session cookie.
  var timer = null;
  function checkUser(delay) {
    clearTimeout(timer);
    timer = setTimeout(function () {
      origFetch.call(window, '/api/auth/me', { credentials: 'include' })
        .then(function (r) {
          if (r.status === 401 || r.status === 403) return { user: null };
          if (!r.ok) return { failed: true };
          return r.json().then(function (u) {
            if (!u || !u.email) return { user: null };
            return origFetch.call(window, '/api/settings', { credentials: 'include' })
              .then(function (s) { return s.ok ? s.json() : null; }, function () { return null; })
              .then(function (s) { return { user: u, settings: s }; });
          });
        }, function () { return { failed: true }; })
        .then(function (r) { return call('frame_user', { user: r.user || null, settings: r.settings || null, failed: !!r.failed }); })
        .catch(function () {});
    }, delay === undefined ? 200 : delay);
  }
  Object.defineProperty(window, '__imagineOneFrameCheck', { value: checkUser });

  window.imagineOneFrame = Object.freeze({
    version: __VERSION__,
    credentials: Object.freeze({
      get: function (slug) { return call('frame_cred_get', { slug: String(slug) }).catch(function () { return null; }); },
      save: function (slug, creds) {
        return call('frame_cred_save', { slug: String(slug), email: String((creds && creds.email) || ''), password: String((creds && creds.password) || '') }).catch(function () {});
      },
      clear: function (slug) { return call('frame_cred_clear', { slug: String(slug) }).catch(function () {}); }
    }),
    /** The app saved theme settings: re-read branding now instead of waiting. */
    refreshBranding: function () { checkUser(0); }
  });

  // Sign-in / sign-out and expired sessions change who is signed in.
  window.fetch = function (input, init) {
    var p = origFetch.apply(this, arguments);
    try {
      var u = new URL(typeof input === 'string' ? input : (input && input.url) || '', location.href);
      if (u.origin === location.origin) {
        var path = u.pathname;
        if (/^\/api\/auth\/(login|logout|verify|callback)/.test(path) || path === '/api/auth/me') p.then(function (r) { if (path !== '/api/auth/me' || r.status === 401) checkUser(); }, function () {});
      }
    } catch (e) {}
    return p;
  };

  // In-app (SPA) navigation: re-read user + theme, at most every 2 s.
  var lastNav = 0;
  function onNav() { var now = Date.now(); if (now - lastNav > 2000) { lastNav = now; checkUser(); } }
  ['pushState', 'replaceState'].forEach(function (m) {
    var orig = history[m];
    history[m] = function () { var r = orig.apply(this, arguments); onNav(); return r; };
  });
  window.addEventListener('popstate', onNav);
  // Theme saved on a page without navigating: every 30 s while visible.
  setInterval(function () { if (document.visibilityState === 'visible') checkUser(0); }, 30000);

  // Sidebar watcher: where the app's sidebar is and its colours, so the title bar continues it.
  var SIDEBAR = 'aside[data-testid="sidebar"], [data-sidebar="sidebar"]';
  var last = '', raf = 0;
  function opaque(c) { return !!c && c !== 'transparent' && !/rgba\(\s*\d+,\s*\d+,\s*\d+,\s*0\)/.test(c); }
  function backgroundOf(el) { for (var n = el; n; n = n.parentElement) { var bg = getComputedStyle(n).backgroundColor; if (opaque(bg)) return bg; } return ''; }
  function measure() {
    raf = 0;
    var el = document.querySelector(SIDEBAR), payload = null;
    if (el) {
      var r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0 && r.right > 0) {
        payload = { left: Math.round(r.left), width: Math.round(r.width), color: backgroundOf(el),
          pageColor: backgroundOf(document.body) || getComputedStyle(document.documentElement).backgroundColor };
      }
    }
    var key = JSON.stringify(payload);
    if (key !== last) { last = key; call('frame_sidebar', { metrics: payload }).catch(function () {}); }
  }
  function schedule() { if (!raf) raf = requestAnimationFrame(measure); }
  function start() {
    new MutationObserver(schedule).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'style', 'data-state', 'data-collapsible'] });
    new ResizeObserver(schedule).observe(document.documentElement);
    window.addEventListener('resize', schedule);
    window.addEventListener('transitionend', schedule, true);
    setInterval(schedule, 1500);
    schedule();
    checkUser(0);
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();

  // Shortcuts the Electron app had in its menu.
  window.addEventListener('keydown', function (e) {
    var k = (e.key || '').toLowerCase(), mod = e.ctrlKey || e.metaKey;
    var hit = function () { e.preventDefault(); e.stopPropagation(); };
    if (mod && !e.shiftKey && k === 'r') { hit(); location.reload(); }
    else if (e.altKey && k === 'arrowleft') { hit(); history.back(); }
    else if (e.altKey && k === 'arrowright') { hit(); history.forward(); }
    else if (e.altKey && k === 'home') { hit(); location.href = '/'; }
    else if (mod && !e.shiftKey && k === 'q') { hit(); call('frame_page_action', { name: 'quit' }).catch(function () {}); }
  }, true);
})();"#;

fn app_js(version: &str) -> String {
    APP_JS.replace("__VERSION__", &serde_json::to_string(version).unwrap_or_else(|_| "\"\"".into()))
}

/// WebView2's own User-Agent plus "ImagineOneFrame/<version>": the server lets the desktop app
/// past the "install the app" (PWA) gate by that token, and keeps the gate for browsers.
fn user_agent(app_version: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let edge = tauri::webview_version().ok()?;
    let major = edge.split('.').next().filter(|m| m.chars().all(|c| c.is_ascii_digit()) && !m.is_empty())?.to_string();
    Some(format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36 Edg/{edge} ImagineOneFrame/{app_version}"
    ))
}

// ---------------------------------------------------------------------------------------------
// Window: compact for the organisation step and sign-in, full size (remembered) once signed in
// ---------------------------------------------------------------------------------------------

fn layout(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    let (Ok(size), Ok(sf)) = (w.inner_size(), w.scale_factor()) else { return };
    let (width, height) = (size.width as f64 / sf, size.height as f64 / sf);
    if let Some(c) = chrome(app) {
        let _ = c.set_bounds(Rect { position: LogicalPosition::new(0.0, 0.0).into(), size: LogicalSize::new(width, height).into() });
    }
    if let Some(v) = app_view(app) {
        let _ = v.set_bounds(Rect {
            position: LogicalPosition::new(0.0, TITLE_BAR).into(),
            size: LogicalSize::new(width, (height - TITLE_BAR).max(0.0)).into(),
        });
    }
}

fn show_app(app: &AppHandle, visible: bool) {
    if let Some(v) = app_view(app) {
        let _ = if visible { v.show() } else { v.hide() };
        if visible {
            layout(app);
            let _ = v.set_focus();
        }
    }
}

fn current_bounds(w: &Window) -> Option<Bounds> {
    let sf = w.scale_factor().ok()?;
    let p = w.outer_position().ok()?.to_logical::<f64>(sf);
    let s = w.inner_size().ok()?.to_logical::<f64>(sf);
    Some(Bounds { x: p.x, y: p.y, width: s.width, height: s.height })
}

fn on_screen(w: &Window, b: &Bounds) -> bool {
    w.available_monitors().unwrap_or_default().iter().any(|m| {
        let sf = m.scale_factor();
        let a = m.work_area();
        let (ax, ay) = (a.position.x as f64 / sf, a.position.y as f64 / sf);
        let (aw, ah) = (a.size.width as f64 / sf, a.size.height as f64 / sf);
        b.x >= ax - 8.0 && b.y >= ay - 8.0 && b.x + b.width <= ax + aw + 8.0 && b.y + b.height <= ay + ah + 8.0
    })
}

fn remember_bounds(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    let st = shared(app);
    let mut c = st.lock().unwrap();
    if c.mode != Mode::Full || w.is_minimized().unwrap_or(false) || !w.is_visible().unwrap_or(true) {
        return;
    }
    let maximized = w.is_maximized().unwrap_or(false);
    let bounds = if maximized { c.normal.or(c.cfg.stored.bounds) } else { current_bounds(&w) };
    c.cfg.stored.bounds = bounds;
    c.cfg.stored.maximized = Some(maximized);
    c.cfg.save();
}

fn set_mode(app: &AppHandle, next: Mode) {
    let Some(w) = main_window(app) else { return };
    if shared(app).lock().unwrap().mode == next {
        return;
    }
    if next == Mode::Compact {
        remember_bounds(app);
        let _ = w.set_fullscreen(false);
        let _ = w.unmaximize();
        let _ = w.set_resizable(false);
        let _ = w.set_maximizable(false);
        let _ = w.set_min_size(Some(LogicalSize::new(COMPACT.0, COMPACT.1)));
        let _ = w.set_size(LogicalSize::new(COMPACT.0, COMPACT.1));
        if let Some(m) = w.current_monitor().ok().flatten() {
            let sf = m.scale_factor();
            let a = m.work_area();
            let (ax, ay) = (a.position.x as f64 / sf, a.position.y as f64 / sf);
            let (aw, ah) = (a.size.width as f64 / sf, a.size.height as f64 / sf);
            let _ = w.set_position(LogicalPosition::new(ax + ((aw - COMPACT.0) / 2.0).max(0.0), ay + ((ah - COMPACT.1) / 2.0).max(0.0)));
        }
    } else {
        let _ = w.set_min_size(Some(LogicalSize::new(FULL_MIN.0, FULL_MIN.1)));
        let _ = w.set_resizable(true);
        let _ = w.set_maximizable(true);
        let (saved, maximized) = {
            let st = shared(app);
            let c = st.lock().unwrap();
            (c.cfg.stored.bounds, c.cfg.stored.maximized.unwrap_or(false))
        };
        match saved.filter(|b| on_screen(&w, b)) {
            Some(b) => {
                let _ = w.set_size(LogicalSize::new(b.width, b.height));
                let _ = w.set_position(LogicalPosition::new(b.x, b.y));
            }
            None => {
                // 92% of the monitor's work area, capped at a comfortable size, centred (as Electron).
                if let Some(m) = w.current_monitor().ok().flatten() {
                    let sf = m.scale_factor();
                    let a = m.work_area();
                    let (aw, ah) = (a.size.width as f64 / sf, a.size.height as f64 / sf);
                    let (width, height) = ((aw * 0.92).min(1500.0), (ah * 0.92).min(1000.0));
                    let _ = w.set_size(LogicalSize::new(width, height));
                    let _ = w.set_position(LogicalPosition::new(a.position.x as f64 / sf + (aw - width) / 2.0, a.position.y as f64 / sf + (ah - height) / 2.0));
                }
            }
        }
        if maximized {
            let _ = w.maximize();
        }
    }
    shared(app).lock().unwrap().mode = next;
    layout(app);
}

// ---------------------------------------------------------------------------------------------
// Server calls (public endpoints only; anything that needs the session runs in the page)
// ---------------------------------------------------------------------------------------------

fn http(app_version: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("ImagineOneFrame/{app_version}"))
        .build()
        .expect("http client")
}

fn describe(e: &reqwest::Error) -> String {
    let mut chain = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        chain.push_str(": ");
        chain.push_str(&s.to_string());
        src = s.source();
    }
    let low = chain.to_lowercase();
    if low.contains("certificate") {
        "The server’s security certificate isn’t valid.".into()
    } else if low.contains("dns") || low.contains("no such host") || low.contains("name or service") {
        "That address could not be found.".into()
    } else if e.is_timeout() {
        "The server didn’t answer in time.".into()
    } else if e.is_connect() {
        "Could not connect to the server.".into()
    } else {
        chain
    }
}

fn app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// GET a public JSON endpoint. Ok(None) for 404.
async fn get_json(app: &AppHandle, url: &str) -> Result<Option<Value>, String> {
    let res = http(&app_version(app))
        .get(url)
        .header("X-ImagineOne-Frame", app_version(app))
        .send()
        .await
        .map_err(|e| describe(&e))?;
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("The server answered {}.", res.status().as_u16()));
    }
    Ok(Some(res.json().await.unwrap_or(Value::Null)))
}

/// Same, cached for 20 s (branding is re-read often while the app is used).
async fn get_cached(app: &AppHandle, url: String) -> Option<Value> {
    if let Some((t, v)) = shared(app).lock().unwrap().cache.get(&url) {
        if t.elapsed() < Duration::from_secs(20) {
            return v.clone();
        }
    }
    let v = get_json(app, &url).await.ok().flatten();
    shared(app).lock().unwrap().cache.insert(url, (Instant::now(), v.clone()));
    v
}

fn tenant_url(base: &str, slug: &str) -> String {
    format!("{base}/api/public/tenant/{}", url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>())
}

/// Re-derive the title bar branding from whatever applies now (src/main/branding.ts resolveBranding).
async fn refresh_branding(app: AppHandle) {
    let (base, cfg_slug, phase, signed_in, settings, product) = {
        let st = shared(&app);
        let c = st.lock().unwrap();
        (c.base(), c.cfg.get().tenant_slug, c.s.phase, c.s.user.is_some(), c.settings.clone(), c.product.clone())
    };
    // On the organisation step no company is chosen yet: brand as the platform.
    let current = if phase == "org" { String::new() } else { app_view(&app).and_then(|v| v.url().ok()).map(|u| u.to_string()).unwrap_or_default() };
    let slug = if cfg_slug.is_empty() { slug_from_url(&current).unwrap_or_default() } else { cfg_slug };
    let tenant = if slug.is_empty() { None } else { get_cached(&app, tenant_url(&base, &slug)).await };
    let mut branding = None;
    if signed_in {
        if let Some(mut b) = settings.as_ref().and_then(|s| from_settings(&base, s, &product)) {
            // The tenant's own public branding is authoritative for identity; settings give colours.
            if let Some(t) = &tenant {
                b.logo_url = absolutize(&base, t.get("logoUrl")).or(b.logo_url);
                if let Some(n) = text(t, "appTitle").or_else(|| text(t, "name")) {
                    b.name = n;
                }
            }
            branding = Some(b);
        }
    }
    if branding.is_none() {
        branding = tenant.as_ref().map(|t| from_tenant(&base, t));
    }
    if branding.is_none() {
        let q = if slug.is_empty() { String::new() } else { format!("?slug={}", url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>()) };
        branding = get_cached(&app, format!("{base}/api/public/site-settings{q}")).await.map(|s| from_site(&base, &s, &product));
    }
    let mut b = branding.unwrap_or_else(|| default_branding(&product));
    if b.name.is_empty() {
        b.name = product;
    }
    if b != state_of(&app).branding {
        push(&app, |s| s.branding = b);
    }
}

/// Give the configured server (only) the web-app commands.
fn allow_server(app: &AppHandle, base: &str) {
    let st = shared(app);
    let mut c = st.lock().unwrap();
    if c.allowed.iter().any(|o| o == base) {
        return;
    }
    let cap = CapabilityBuilder::new(format!("imagineone-server-{}", c.allowed.len() + 1))
        .remote(format!("{base}/*"))
        .webview("app")
        .permission("allow-frame-user")
        .permission("allow-frame-sidebar")
        .permission("allow-frame-cred-get")
        .permission("allow-frame-cred-save")
        .permission("allow-frame-cred-clear")
        .permission("allow-frame-page-action");
    if app.add_capability(cap).is_ok() {
        c.allowed.push(base.to_string());
    }
}

fn load(app: &AppHandle, url: &str) {
    if let (Some(v), Ok(u)) = (app_view(app), Url::parse(url)) {
        let _ = v.navigate(u);
    }
}

// ---------------------------------------------------------------------------------------------
// Boot (frame-host-api-reference.md):
//   1. GET /api/public/deployment   reachability
//   2. GET /api/public/tenant/:slug the install lock (when a company is configured)
//   3. the page says whether someone is signed in → the app, the sign-in page, or the org step
// ---------------------------------------------------------------------------------------------

async fn boot(app: AppHandle) {
    show_app(&app, false);
    set_mode(&app, Mode::Compact);
    let id = {
        let st = shared(&app);
        let mut c = st.lock().unwrap();
        c.boot_id += 1;
        c.awaiting_user = false;
        c.settings = None;
        c.boot_id
    };
    let cfg = shared(&app).lock().unwrap().cfg.get();
    push(&app, |s| {
        s.phase = "starting";
        s.error_message = None;
        s.org_error = None;
        s.org_busy = false;
        s.user = None;
        s.sidebar = None;
        s.config = cfg.clone();
    });
    let base = cfg.base_url.clone();
    if let Err(e) = get_json(&app, &format!("{base}/api/public/deployment")).await.and_then(|v| v.ok_or_else(|| "The server answered 404.".to_string())) {
        push(&app, |s| {
            s.phase = "config-error";
            s.error_message = Some(format!("Cannot reach {base}. Deployment check failed: {e}"));
        });
        return;
    }
    let slug = cfg.tenant_slug.clone();
    if !slug.is_empty() {
        match get_json(&app, &tenant_url(&base, &slug)).await {
            Err(e) => {
                push(&app, |s| {
                    s.phase = "config-error";
                    s.error_message = Some(format!("Tenant lookup failed: {e}"));
                });
                return;
            }
            Ok(t) if t.as_ref().and_then(|t| text(t, "slug")).map(|s| s.to_lowercase()) != Some(slug.clone()) => {
                push(&app, |s| {
                    s.phase = "config-error";
                    s.error_message = Some(format!("This install is configured for company \"{slug}\", but {base} does not serve that company."));
                });
                return;
            }
            Ok(_) => {}
        }
    }
    allow_server(&app, &base);
    // Load the page hidden: it reports whether someone is still signed in from last time.
    shared(&app).lock().unwrap().awaiting_user = true;
    let start = if slug.is_empty() { format!("{base}/") } else { format!("{base}/t/{}", url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>()) };
    load(&app, &start);
    // If the page never answers (it didn't load), carry on as signed out.
    let later = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let still = {
            let st = shared(&later);
            let c = st.lock().unwrap();
            c.boot_id == id && c.awaiting_user
        };
        if still {
            decide(&later, None);
        }
    });
}

fn reboot(app: &AppHandle) {
    tauri::async_runtime::spawn(boot(app.clone()));
}

/// The organisation step (compact window, drawn by the title bar page).
fn enter_org(app: &AppHandle) {
    set_mode(app, Mode::Compact);
    show_app(app, false);
    push(app, |s| {
        s.phase = "org";
        s.org_error = None;
        s.org_busy = false;
        s.user = None;
        s.sidebar = None;
    });
    tauri::async_runtime::spawn(refresh_branding(app.clone()));
}

/// The app's own /t/:slug sign-in page (compact window).
fn enter_login(app: &AppHandle, slug: &str) {
    set_mode(app, Mode::Compact);
    push(app, |s| {
        s.phase = "login";
        s.org_error = None;
        s.org_busy = false;
        s.user = None;
        s.sidebar = None;
    });
    let base = shared(app).lock().unwrap().base();
    let want = format!("{base}/t/{}", url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>());
    let here = app_view(app).and_then(|v| v.url().ok());
    if !here.as_ref().is_some_and(|u| u.as_str().trim_end_matches('/') == want) {
        load(app, &want);
    }
    show_app(app, true);
    tauri::async_runtime::spawn(refresh_branding(app.clone()));
}

/// Signed in: the full window.
fn enter_app(app: &AppHandle) {
    set_mode(app, Mode::Full);
    push(app, |s| {
        s.phase = "ready";
        s.org_error = None;
        s.org_busy = false;
        s.can_go_back = true;
        s.can_go_forward = true;
    });
    let base = shared(app).lock().unwrap().base();
    let here = app_view(app).and_then(|v| v.url().ok());
    if here.as_ref().is_none_or(|u| is_login_path(u) || !is_allowed(&base, u)) {
        load(app, &format!("{base}/"));
    }
    show_app(app, true);
    tauri::async_runtime::spawn(refresh_branding(app.clone()));
}

/// Where to go once we know who is signed in (refreshUser in the Electron app).
fn decide(app: &AppHandle, user: Option<Value>) {
    let (awaiting, phase, slug) = {
        let st = shared(app);
        let mut c = st.lock().unwrap();
        let a = c.awaiting_user;
        c.awaiting_user = false;
        (a, c.s.phase, c.cfg.get().tenant_slug)
    };
    let signed_in = user.is_some();
    if user != state_of(app).user {
        push(app, |s| s.user = user);
    }
    if awaiting {
        if signed_in {
            enter_app(app);
        } else if !slug.is_empty() {
            enter_login(app, &slug);
        } else {
            enter_org(app);
        }
        return;
    }
    match (phase, signed_in) {
        ("login", true) | ("org", true) => enter_app(app),
        ("ready", false) => {
            if slug.is_empty() {
                enter_org(app)
            } else {
                enter_login(app, &slug)
            }
        }
        _ => {
            tauri::async_runtime::spawn(refresh_branding(app.clone()));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Actions (☰ menu, title bar, tray)
// ---------------------------------------------------------------------------------------------

fn navigate(app: &AppHandle, path: &str) {
    let base = shared(app).lock().unwrap().base();
    let url = if path.starts_with("http://") || path.starts_with("https://") { path.to_string() } else { format!("{base}{path}") };
    let Ok(u) = Url::parse(&url) else { return };
    if is_allowed(&base, &u) {
        load(app, u.as_str());
    } else {
        open_externally(app, &u);
    }
}

fn can_change_setup(app: &AppHandle) -> bool {
    shared(app).lock().unwrap().is_admin()
}

fn action(app: &AppHandle, name: &str) {
    match name {
        "home" => navigate(app, "/"),
        "reload" => {
            if state_of(app).phase == "config-error" {
                reboot(app);
            } else if let Some(v) = app_view(app) {
                let _ = v.reload();
            }
        }
        "retry" => reboot(app),
        "back" => {
            if let Some(v) = app_view(app) {
                let _ = v.eval("history.back()");
            }
        }
        "forward" => {
            if let Some(v) = app_view(app) {
                let _ = v.eval("history.forward()");
            }
        }
        "signOut" => {
            // The session cookie is the page's: sign out from the page, which then reports "nobody".
            if let Some(v) = app_view(app) {
                let _ = v.eval(
                    "fetch('/api/auth/logout',{method:'POST',credentials:'include'}).catch(function(){}).then(function(){ if (window.__imagineOneFrameCheck) window.__imagineOneFrameCheck(0); });",
                );
            }
        }
        "changeOrg" => {
            if state_of(app).user.is_some() && !can_change_setup(app) {
                return;
            }
            {
                let st = shared(app);
                let mut c = st.lock().unwrap();
                c.cfg.set(None, Some(""));
                let cfg = c.cfg.get();
                c.s.config = cfg;
            }
            enter_org(app);
        }
        "openSettings" => {
            if !can_change_setup(app) {
                return;
            }
            show_app(app, false);
            if let Some(c) = chrome(app) {
                let _ = c.eval("window.__frameOpenSettings && window.__frameOpenSettings()");
            }
        }
        "checkForUpdates" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check_for_update(app, true).await });
        }
        "about" => {
            let s = state_of(app);
            app.dialog()
                .message(format!(
                    "Desktop app {}\nTauri {} · WebView2 {}\n\n{}",
                    s.app_version,
                    tauri::VERSION,
                    tauri::webview_version().unwrap_or_default(),
                    s.config.base_url
                ))
                .title(format!("About {}", s.branding.name))
                .show(|_| {});
        }
        "show" => show_window(app),
        "quit" => {
            shared(app).lock().unwrap().quitting = true;
            remember_bounds(app);
            app.exit(0);
        }
        _ => {}
    }
}

fn show_window(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The ☰ menu (the Electron app's native menu): Administration only for admins.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let s = state_of(app);
    let ready = s.phase == "ready" && s.user.is_some();
    let is_admin = shared(app).lock().unwrap().is_admin();
    let email = s.user.as_ref().and_then(|u| u.get("email")).and_then(|e| e.as_str()).unwrap_or("").to_string();
    let console = s.user.as_ref().and_then(|u| u.get("selfHostedConsoleUrl")).and_then(|e| e.as_str()).unwrap_or("").to_string();
    let item = |id: &str, text: &str, enabled: bool, accel: Option<&str>| MenuItem::with_id(app, id, text, enabled, accel);
    let sep = || PredefinedMenuItem::separator(app);

    let menu = Menu::new(app)?;
    let main = Submenu::new(app, &s.branding.name, true)?;
    main.append(&item("a:home", "Home", ready, Some("Alt+Home"))?)?;
    main.append(&item("a:reload", "Reload", true, Some("CmdOrCtrl+R"))?)?;
    main.append(&item("a:back", "Back", ready, Some("Alt+Left"))?)?;
    main.append(&item("a:forward", "Forward", ready, Some("Alt+Right"))?)?;
    main.append(&sep()?)?;
    if s.user.is_some() {
        main.append(&item("a:signOut", &format!("Sign out ({email})"), true, None)?)?;
    }
    if !s.config.tenant_slug.is_empty() && s.user.is_none() {
        main.append(&item("a:changeOrg", &format!("Change organisation… ({})", s.config.tenant_slug), true, None)?)?;
    }
    main.append(&sep()?)?;
    if is_admin {
        main.append(&item("a:openSettings", "Frame settings…", true, None)?)?;
    }
    main.append(&item("a:checkForUpdates", "Check for updates…", true, None)?)?;
    main.append(&item("a:about", "About", true, None)?)?;
    main.append(&sep()?)?;
    main.append(&item("a:quit", "Quit", true, Some("CmdOrCtrl+Q"))?)?;
    menu.append(&main)?;

    if ready && is_admin {
        let adm = Submenu::new(app, "Administration", true)?;
        adm.append(&item("go:/admin/settings", "Settings", true, None)?)?;
        adm.append(&item("go:/admin/users", "Users", true, None)?)?;
        adm.append(&item("go:/admin/software-update", "Software Update", true, None)?)?;
        if !console.is_empty() {
            adm.append(&sep()?)?;
            adm.append(&item("console", "Open deployment console", true, None)?)?;
        }
        menu.append(&adm)?;
    }

    let edit = Submenu::new(app, "Edit", true)?;
    edit.append(&PredefinedMenuItem::undo(app, None)?)?;
    edit.append(&PredefinedMenuItem::redo(app, None)?)?;
    edit.append(&sep()?)?;
    edit.append(&PredefinedMenuItem::cut(app, None)?)?;
    edit.append(&PredefinedMenuItem::copy(app, None)?)?;
    edit.append(&PredefinedMenuItem::paste(app, None)?)?;
    edit.append(&PredefinedMenuItem::select_all(app, None)?)?;
    menu.append(&edit)?;

    let view = Submenu::new(app, "View", true)?;
    view.append(&item("zoom:reset", "Actual size", true, Some("CmdOrCtrl+0"))?)?;
    view.append(&item("zoom:in", "Zoom in", true, Some("CmdOrCtrl+Plus"))?)?;
    view.append(&item("zoom:out", "Zoom out", true, Some("CmdOrCtrl+-"))?)?;
    view.append(&sep()?)?;
    view.append(&item("win:fullscreen", "Toggle full screen", ready, Some("F11"))?)?;
    menu.append(&view)?;
    let win = Submenu::new(app, "Window", true)?;
    win.append(&item("win:minimize", "Minimize", true, None)?)?;
    win.append(&item("win:close", "Close", true, None)?)?;
    menu.append(&win)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, id: &str) {
    if let Some(path) = id.strip_prefix("go:") {
        navigate(app, path);
    } else if id == "console" {
        let url = state_of(app).user.and_then(|u| u.get("selfHostedConsoleUrl").and_then(|e| e.as_str()).map(String::from)).unwrap_or_default();
        if !url.is_empty() {
            navigate(app, &url);
        }
    } else if let Some(a) = id.strip_prefix("a:") {
        action(app, a);
    } else if let Some(t) = id.strip_prefix("tray:") {
        action(app, t);
    } else if let Some(z) = id.strip_prefix("zoom:") {
        let level = {
            let st = shared(app);
            let mut c = st.lock().unwrap();
            c.zoom = match z {
                "in" => (c.zoom + 0.1).min(3.0),
                "out" => (c.zoom - 0.1).max(0.5),
                _ => 1.0,
            };
            c.zoom
        };
        if let Some(v) = app_view(app) {
            let _ = v.set_zoom(level);
        }
    } else if let Some(w) = main_window(app) {
        match id {
            "win:fullscreen" => {
                let full = w.is_fullscreen().unwrap_or(false);
                let _ = w.set_fullscreen(!full);
                layout(app);
            }
            "win:minimize" => {
                let _ = w.minimize();
            }
            "win:close" => {
                let _ = w.close();
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// "Remember me" for the web app's sign-in page, per organisation: Windows data protection (DPAPI),
// so only this Windows user on this PC can read it.
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct SavedLogin {
    email: String,
    /// base64 of the DPAPI-protected password
    secret: String,
}

fn creds_file(app: &AppHandle) -> PathBuf {
    shared(app).lock().unwrap().cfg.dir.join("credentials-v2.json")
}

fn read_creds(app: &AppHandle) -> HashMap<String, SavedLogin> {
    fs::read_to_string(creds_file(app)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_creds(app: &AppHandle, all: &HashMap<String, SavedLogin>) -> bool {
    let path = creds_file(app);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    serde_json::to_string(all).ok().is_some_and(|t| fs::write(path, t).is_ok())
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};

    fn run(input: &[u8], protect: bool) -> Option<Vec<u8>> {
        let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
        let mut blob_out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            if protect {
                CryptProtectData(&blob_in, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut blob_out)
            } else {
                CryptUnprotectData(&blob_in, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut blob_out)
            }
        };
        if ok == 0 || blob_out.pbData.is_null() {
            return None;
        }
        let out = unsafe { std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec() };
        unsafe { LocalFree(blob_out.pbData as _) };
        Some(out)
    }
    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, true)
    }
    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, false)
    }
}
#[cfg(not(windows))]
mod dpapi {
    // Only Windows has a place to keep the password safely here; elsewhere "remember" is off.
    pub fn protect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
    pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Updates from GitHub releases (Desktop.ps1 Publish): the feed address is set at build time
// (tauri.conf.json plugins.updater.endpoints), installers are signed with our update key, and
// the app asks before restarting.
// ---------------------------------------------------------------------------------------------

fn updater_setting(app: &AppHandle, key: &str) -> Option<Value> {
    app.config().plugins.0.get("updater").and_then(|u| u.get(key)).cloned()
}

fn can_update(app: &AppHandle) -> bool {
    let key = updater_setting(app, "pubkey").and_then(|k| k.as_str().map(|s| s.trim().to_string())).unwrap_or_default();
    let endpoints = updater_setting(app, "endpoints").and_then(|e| e.as_array().map(|a| a.len())).unwrap_or(0);
    !key.is_empty() && endpoints > 0
}

async fn check_for_update(app: AppHandle, manual: bool) {
    use tauri_plugin_updater::UpdaterExt;
    {
        let st = shared(&app);
        let mut c = st.lock().unwrap();
        if c.updating {
            return;
        }
        c.updating = true;
    }
    let name = state_of(&app).branding.name;
    let result: Result<Option<String>, String> = async {
        if !can_update(&app) {
            return Err("This build doesn’t update itself (it was built without the update key).".into());
        }
        let updater = app.updater_builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
        let update = match updater.check().await {
            Ok(Some(u)) => u,
            Ok(None) => return Ok(Some(format!("{name} is up to date ({}).", app.package_info().version))),
            Err(tauri_plugin_updater::Error::ReleaseNotFound) => return Err("No updates are published yet.".into()),
            Err(e) => return Err(format!("Couldn’t check for updates: {e}")),
        };
        let yes = app
            .dialog()
            .message(format!("Version {} of {name} is available. Install it now? The app restarts.", update.version))
            .title("Update")
            .buttons(MessageDialogButtons::OkCancelCustom("Install now".into(), "Later".into()))
            .blocking_show();
        if !yes {
            return Ok(None);
        }
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| format!("The update to {} didn’t install: {e}", update.version))?;
        app.restart();
    }
    .await;
    shared(&app).lock().unwrap().updating = false;
    let msg = match result {
        Ok(Some(m)) if manual => Some(m),
        Err(m) if manual => Some(m),
        _ => None,
    };
    if let Some(m) = msg {
        app.dialog().message(m).title(name).show(|_| {});
    }
}

// ---------------------------------------------------------------------------------------------
// Commands: the title bar (local page "chrome")
// ---------------------------------------------------------------------------------------------

fn from_chrome(w: &Webview) -> bool {
    w.label() == "chrome" && w.url().ok().is_some_and(|u| is_local(&u))
}
/// The web app itself: our app webview, on the configured server.
fn from_app(app: &AppHandle, w: &Webview) -> bool {
    w.label() == "app" && w.url().ok().is_some_and(|u| shared(app).lock().unwrap().is_server(&u))
}

#[tauri::command]
fn frame_get_state(app: AppHandle, webview: Webview) -> Result<FrameState, String> {
    if !from_chrome(&webview) {
        return Err("Not allowed".into());
    }
    Ok(state_of(&app))
}

#[tauri::command]
fn frame_action(app: AppHandle, webview: Webview, name: String) {
    const OK: [&str; 11] = ["home", "reload", "retry", "back", "forward", "signOut", "changeOrg", "openSettings", "checkForUpdates", "about", "quit"];
    if from_chrome(&webview) && OK.contains(&name.as_str()) {
        action(&app, &name);
    }
}

#[tauri::command]
async fn frame_continue_org(app: AppHandle, webview: Webview, slug: String) -> Result<(), String> {
    if !from_chrome(&webview) {
        return Err("Not allowed".into());
    }
    let slug: String = slug.trim().to_lowercase().chars().take(80).collect();
    if slug.is_empty() {
        push(&app, |s| s.org_error = Some("Enter your organisation identifier.".into()));
        return Ok(());
    }
    if clean_slug(&slug) != slug {
        push(&app, |s| s.org_error = Some(format!("No organisation \"{slug}\" was found. Check the identifier from your administrator.")));
        return Ok(());
    }
    push(&app, |s| {
        s.org_busy = true;
        s.org_error = None;
    });
    let base = shared(&app).lock().unwrap().base();
    match get_json(&app, &tenant_url(&base, &slug)).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            push(&app, |s| {
                s.org_busy = false;
                s.org_error = Some(format!("No organisation \"{slug}\" was found. Check the identifier from your administrator."));
            });
            return Ok(());
        }
        Err(e) => {
            push(&app, |s| {
                s.org_busy = false;
                s.org_error = Some(format!("Could not verify the organisation: {e}"));
            });
            return Ok(());
        }
    }
    {
        let st = shared(&app);
        let mut c = st.lock().unwrap();
        c.cfg.set(None, Some(&slug));
        let cfg = c.cfg.get();
        c.s.config = cfg;
    }
    allow_server(&app, &base);
    enter_login(&app, &slug);
    Ok(())
}

/// Frame settings (server address, company). Admins only, and on the "configuration problem"
/// screen, where nobody can be signed in (the Electron app refused it there, so it couldn't be fixed).
#[tauri::command]
fn frame_save_settings(app: AppHandle, webview: Webview, base_url: Option<String>, tenant_slug: Option<String>) -> Result<(), String> {
    if !from_chrome(&webview) {
        return Err("Not allowed".into());
    }
    let phase = state_of(&app).phase;
    if !(can_change_setup(&app) || phase == "config-error" || phase == "starting") {
        return Err("Only an administrator can change the frame settings.".into());
    }
    let base: Option<String> = base_url.map(|b| b.chars().take(300).collect());
    let slug: Option<String> = tenant_slug.map(|t| t.chars().take(80).collect());
    {
        let st = shared(&app);
        let mut c = st.lock().unwrap();
        c.cfg.set(base.as_deref(), slug.as_deref());
    }
    reboot(&app);
    Ok(())
}

#[tauri::command]
fn frame_settings_closed(app: AppHandle, webview: Webview) {
    if from_chrome(&webview) && matches!(state_of(&app).phase, "ready" | "login") {
        show_app(&app, true);
    }
}

#[tauri::command]
fn frame_open_menu(app: AppHandle, webview: Webview, x: f64, y: f64) {
    if !from_chrome(&webview) {
        return;
    }
    if let (Some(w), Ok(menu)) = (main_window(&app), build_menu(&app)) {
        let _ = w.popup_menu_at(&menu, LogicalPosition::new(x.round(), y.round()));
    }
}

#[tauri::command]
fn frame_online(app: AppHandle, webview: Webview, online: bool) {
    if from_chrome(&webview) && state_of(&app).online != online {
        push(&app, |s| s.online = online);
    }
}

#[tauri::command]
fn frame_win(app: AppHandle, webview: Webview, what: String) {
    if !from_chrome(&webview) {
        return;
    }
    let Some(w) = main_window(&app) else { return };
    match what.as_str() {
        "minimize" => {
            let _ = w.minimize();
        }
        "toggleMaximize" if w.is_maximizable().unwrap_or(false) => {
            let _ = if w.is_maximized().unwrap_or(false) { w.unmaximize() } else { w.maximize() };
        }
        "close" => {
            let _ = w.close();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Commands: the web app (configured server only)
// ---------------------------------------------------------------------------------------------

/// The page reports who is signed in, with the tenant's settings (on load, sign-in, sign-out,
/// in-app navigation, every 30 s). `failed` = it couldn't ask (offline): nothing changes.
#[tauri::command]
fn frame_user(app: AppHandle, webview: Webview, user: Option<Value>, settings: Option<Value>, failed: Option<bool>) {
    if !from_app(&app, &webview) || failed.unwrap_or(false) {
        return;
    }
    let user = user.filter(|u| u.is_object() && u.get("email").and_then(|e| e.as_str()).is_some_and(|e| !e.is_empty() && e.len() < 300));
    let settings = settings.filter(|s| s.is_object());
    shared(&app).lock().unwrap().settings = if user.is_some() { settings } else { None };
    decide(&app, user);
}

#[tauri::command]
fn frame_sidebar(app: AppHandle, webview: Webview, metrics: Option<Sidebar>) {
    if !from_app(&app, &webview) {
        return;
    }
    let m = metrics.filter(|m| m.width.is_finite() && m.left.is_finite() && m.width > 0.0 && m.color.len() < 100 && m.page_color.len() < 100);
    if m != state_of(&app).sidebar {
        push(&app, |s| s.sidebar = m);
    }
}

#[derive(Serialize)]
struct Login {
    email: String,
    password: String,
}

#[tauri::command]
fn frame_cred_get(app: AppHandle, webview: Webview, slug: String) -> Option<Login> {
    if !from_app(&app, &webview) {
        return None;
    }
    let f = read_creds(&app).remove(&clean_slug(&slug))?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(f.secret).ok()?;
    let password = String::from_utf8(dpapi::unprotect(&bytes)?).ok()?;
    (!f.email.is_empty() && !password.is_empty()).then_some(Login { email: f.email, password })
}

#[tauri::command]
fn frame_cred_save(app: AppHandle, webview: Webview, slug: String, email: String, password: String) -> bool {
    let slug = clean_slug(&slug);
    if !from_app(&app, &webview) || slug.is_empty() || email.is_empty() || password.is_empty() || email.len() > 200 || password.len() > 200 {
        return false;
    }
    let Some(secret) = dpapi::protect(password.as_bytes()) else { return false };
    use base64::Engine;
    let mut all = read_creds(&app);
    all.insert(slug, SavedLogin { email, secret: base64::engine::general_purpose::STANDARD.encode(secret) });
    write_creds(&app, &all)
}

#[tauri::command]
fn frame_cred_clear(app: AppHandle, webview: Webview, slug: String) {
    if from_app(&app, &webview) {
        let mut all = read_creds(&app);
        if all.remove(&clean_slug(&slug)).is_some() {
            write_creds(&app, &all);
        }
    }
}

#[tauri::command]
fn frame_page_action(app: AppHandle, webview: Webview, name: String) {
    if from_app(&app, &webview) && name == "quit" {
        action(&app, "quit");
    }
}

// ---------------------------------------------------------------------------------------------
// Sign-in popup for Google / Microsoft: shares the app's cookies; when the provider sends the user
// back to our server, the popup closes and the app follows the callback.
// ---------------------------------------------------------------------------------------------

fn open_auth_popup(app: &AppHandle, url: Url) {
    if let Some(p) = app.get_webview_window("auth") {
        let _ = p.navigate(url);
        let _ = p.set_focus();
        return;
    }
    let nav_app = app.clone();
    let load_app = app.clone();
    let mut b = WebviewWindowBuilder::new(app, "auth", WebviewUrl::External(url))
        .title("Sign in")
        .inner_size(520.0, 700.0)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .center()
        .on_navigation(move |u| {
            let base = shared(&nav_app).lock().unwrap().base();
            if is_allowed(&base, u) || is_auth_provider(u) {
                return true;
            }
            open_externally(&nav_app, u);
            false
        })
        .on_page_load(move |w, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let back = shared(&load_app).lock().unwrap().is_server(payload.url());
            if back {
                // Back on our server: the session cookie is set. Move the page into the app.
                let target = payload.url().clone();
                let app = load_app.clone();
                let _ = w.close();
                load(&app, target.as_str());
            }
        });
    if let Some(ua) = user_agent(&app_version(app)) {
        b = b.user_agent(&ua);
    }
    let _ = b.build();
}

fn open_externally(app: &AppHandle, url: &Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

// ---------------------------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------------------------

fn create_window(app: &AppHandle) -> tauri::Result<()> {
    let name = shared(app).lock().unwrap().product.clone();
    let version = app_version(app);
    let w = WindowBuilder::new(app, "main")
        .title(&name)
        .inner_size(COMPACT.0, COMPACT.1)
        .resizable(false)
        .maximizable(false)
        .center()
        .decorations(false) // our title bar draws the buttons
        .shadow(true)
        .transparent(true)
        .build()?;
    // Glass behind the title bar, as in the Electron app (Windows 11 acrylic).
    #[cfg(windows)]
    let _ = w.set_effects(WindowEffectsConfig { effects: vec![Effect::Acrylic], ..Default::default() });
    #[cfg(not(windows))]
    let _ = (&w, Effect::Acrylic, WindowEffectsConfig::default());

    let bar = WebviewBuilder::new("chrome", WebviewUrl::App("index.html".into()))
        .transparent(true)
        .on_navigation(|url| is_local(url)) // the title bar never navigates away
        .on_new_window({
            let app = app.clone();
            move |url, _| {
                open_externally(&app, &url);
                NewWindowResponse::Deny
            }
        });
    w.add_child(bar, LogicalPosition::new(0.0, 0.0), LogicalSize::new(COMPACT.0, COMPACT.1))?;

    let nav_app = app.clone();
    let win_app = app.clone();
    let perm_app = app.clone();
    let dl_app = app.clone();
    let mut page = WebviewBuilder::new("app", WebviewUrl::App("blank.html".into()))
        .initialization_script(app_js(&version))
        .background_color(Color(0xFF, 0xFF, 0xFF, 0xFF))
        .zoom_hotkeys_enabled(true)
        .on_navigation(move |url| {
            if is_local(url) {
                return true;
            }
            let base = shared(&nav_app).lock().unwrap().base();
            if is_allowed(&base, url) {
                // A new page: the sidebar is measured again once it is there.
                if state_of(&nav_app).sidebar.is_some() {
                    push(&nav_app, |s| s.sidebar = None);
                }
                return true;
            }
            // The app signs in with Google / Microsoft by a full-page redirect: run it in the popup.
            if is_auth_provider(url) {
                open_auth_popup(&nav_app, url.clone());
            } else {
                open_externally(&nav_app, url);
            }
            false
        })
        .on_new_window(move |url, _| {
            let base = shared(&win_app).lock().unwrap().base();
            if is_auth_provider(&url) {
                open_auth_popup(&win_app, url);
            } else if is_allowed(&base, &url) {
                // target=_blank links to our own server stay inside the app.
                load(&win_app, url.as_str());
            } else {
                open_externally(&win_app, &url);
            }
            NewWindowResponse::Deny
        })
        .on_permission_request(move |w, kind| {
            let ok = w.url().ok().is_some_and(|u| shared(&perm_app).lock().unwrap().is_server(&u));
            match kind {
                PermissionKind::Microphone | PermissionKind::Notifications | PermissionKind::ClipboardRead | PermissionKind::Autoplay if ok => PermissionResponse::Allow,
                _ => PermissionResponse::Default,
            }
        })
        // Downloads go to the Downloads folder and are shown in Explorer when done.
        .on_download(move |_, event| {
            if let DownloadEvent::Finished { path: Some(p), success: true, .. } = event {
                let _ = dl_app.opener().reveal_item_in_dir(p);
            }
            true
        });
    if let Some(ua) = user_agent(&version) {
        page = page.user_agent(&ua);
    }
    let v = w.add_child(page, LogicalPosition::new(0.0, TITLE_BAR), LogicalSize::new(COMPACT.0, COMPACT.1 - TITLE_BAR))?;
    let _ = v.hide();

    let ev_app = app.clone();
    w.on_window_event(move |e| match e {
        WindowEvent::Resized(_) => {
            layout(&ev_app);
            if let Some(w) = main_window(&ev_app) {
                let max = w.is_maximized().unwrap_or(false);
                let st = shared(&ev_app);
                let mut c = st.lock().unwrap();
                if c.mode == Mode::Full && !max && !w.is_minimized().unwrap_or(false) {
                    c.normal = current_bounds(&w);
                }
                let changed = c.s.maximized != max;
                drop(c);
                if changed {
                    push(&ev_app, |s| s.maximized = max);
                }
            }
        }
        WindowEvent::Moved(_) => {
            if let Some(w) = main_window(&ev_app) {
                let st = shared(&ev_app);
                let mut c = st.lock().unwrap();
                if c.mode == Mode::Full && !w.is_maximized().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
                    c.normal = current_bounds(&w);
                }
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            remember_bounds(&ev_app);
            // Close = keep running in the tray (Quit in the menu or the tray really quits).
            if !shared(&ev_app).lock().unwrap().quitting {
                api.prevent_close();
                if let Some(w) = main_window(&ev_app) {
                    let _ = w.hide();
                }
            }
        }
        _ => {}
    });
    Ok(())
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let name = shared(app).lock().unwrap().product.clone();
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "tray:show", format!("Open {name}"), true, None::<&str>)?,
            &MenuItem::with_id(app, "tray:home", "Home", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "tray:checkForUpdates", "Check for updates", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "tray:quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip(&name).menu(&menu).show_menu_on_left_click(false).on_tray_icon_event(|tray, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_window(tray.app_handle());
        }
    });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        // A second start brings the running app to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_window(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            frame_get_state,
            frame_action,
            frame_continue_org,
            frame_save_settings,
            frame_settings_closed,
            frame_open_menu,
            frame_online,
            frame_win,
            frame_user,
            frame_sidebar,
            frame_cred_get,
            frame_cred_save,
            frame_cred_clear,
            frame_page_action
        ])
        .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
        .setup(|app| {
            let handle = app.handle().clone();
            let product = app.package_info().name.clone();
            let packaged = app
                .path()
                .resolve("resources/frame-config.json", BaseDirectory::Resource)
                .ok()
                .and_then(|p| fs::read_to_string(p).ok())
                .map(|raw| raw.trim_start_matches('\u{feff}').to_string())
                .and_then(|raw| serde_json::from_str::<Packaged>(&raw).ok().map(|p| (raw, p)))
                .filter(|(_, p)| p.base_url.is_some() || p.tenant_slug.is_some());
            // Per customer: their own settings folder (the same one the Electron app used).
            let slug = Config::packaged_slug(&packaged);
            let dir = if slug.is_empty() { app.path().data_dir()?.join(&product) } else { app.path().data_dir()?.join("ImagineOne").join(&slug) };
            let stored = fs::read_to_string(dir.join("frame-config.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Stored>(t.trim_start_matches('\u{feff}')).ok())
                .unwrap_or_default();
            let dev_server = if cfg!(debug_assertions) { std::env::var("I1FRAME_SERVER").ok().filter(|s| !s.trim().is_empty()) } else { None };
            let mut cfg = Config { dir, packaged, stored, dev_server };
            cfg.apply_packaged_defaults();
            let config = cfg.get();
            app.manage::<Shared>(Arc::new(Mutex::new(Frame {
                cfg,
                s: FrameState {
                    phase: "starting",
                    sidebar: None,
                    config,
                    branding: default_branding(&product),
                    user: None,
                    error_message: None,
                    org_error: None,
                    org_busy: false,
                    online: true,
                    can_go_back: false,
                    can_go_forward: false,
                    maximized: false,
                    app_version: app.package_info().version.to_string(),
                    platform: platform(),
                },
                product,
                mode: Mode::Compact,
                allowed: Vec::new(),
                awaiting_user: false,
                boot_id: 0,
                settings: None,
                cache: HashMap::new(),
                zoom: 1.0,
                updating: false,
                quitting: false,
                normal: None,
            })));
            create_window(&handle)?;
            let _ = create_tray(&handle);
            reboot(&handle);
            let upd = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(15)).await;
                loop {
                    check_for_update(upd.clone(), false).await;
                    tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the desktop app");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_urls() {
        assert_eq!(normalize_base_url("app.imagineone.it"), "https://app.imagineone.it");
        assert_eq!(normalize_base_url(" https://nefco.imagineone.it/t/nefco/ "), "https://nefco.imagineone.it");
        assert_eq!(normalize_base_url("http://localhost:5000/x"), "http://localhost:5000");
        assert_eq!(normalize_base_url(""), DEFAULT_BASE);
        assert_eq!(normalize_base_url("https://"), DEFAULT_BASE);
    }

    #[test]
    fn navigation_scope() {
        let base = "https://nefco.imagineone.it";
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(is_allowed(base, &u("https://nefco.imagineone.it/projects")));
        assert!(is_allowed(base, &u("http://nefco.imagineone.it/x"))); // same host, as Electron
        assert!(!is_allowed(base, &u("https://evil.example/nefco.imagineone.it")));
        assert!(!is_allowed(base, &u("https://nefco.imagineone.it:8443/")));
        assert!(!is_allowed(base, &u("mailto:x@y.z")));
        assert!(is_auth_provider(&u("https://accounts.google.com/o/oauth2/auth")));
        assert!(is_auth_provider(&u("https://login.microsoftonline.com/common/oauth2")));
        assert!(!is_auth_provider(&u("http://accounts.google.com/")));
        assert!(!is_auth_provider(&u("https://notaccounts.google.com.evil.io/")));
        assert_eq!(slug_from_url("https://x.it/t/NEFCO/login").as_deref(), Some("nefco"));
        assert_eq!(slug_from_url("https://x.it/projects"), None);
        assert!(is_login_path(&u("https://x.it/t/nefco")));
        assert!(is_login_path(&u("https://x.it/t/nefco/")));
        assert!(!is_login_path(&u("https://x.it/t/nefco/projects")));
    }

    #[test]
    fn branding_rules() {
        let base = "https://x.it";
        let t = from_tenant(base, &json!({ "name": "NEFCO", "appTitle": "", "logoUrl": "/uploads/logo.png", "primaryColor": "#fff", "accentColor": "#123456" }));
        assert_eq!(t.name, "NEFCO");
        assert_eq!(t.logo_url.as_deref(), Some("https://x.it/uploads/logo.png"));
        assert_eq!(t.bar_background, "#ffffff");
        assert_eq!(t.bar_foreground, "#1f2328");
        assert_eq!(t.primary_color, "#123456");
        let s = from_settings(base, &json!({ "company-name": "Acme", "theme-titlebar-background-color": "#000000" }), "Imagine One IT").unwrap();
        assert!(s.bar_explicit);
        assert_eq!((s.bar_background.as_str(), s.bar_foreground.as_str()), ("#000000", "#ffffff"));
        let s = from_settings(base, &json!({}), "Imagine One IT").unwrap();
        assert_eq!(s.bar_background, APP_DEFAULT_SIDEBAR);
        assert_eq!(s.name, "Imagine One IT");
        let site = from_site(base, &json!({ "siteName": "Imagine One", "primaryColor": "javascript:x", "logoUrl": "javascript:alert(1)" }), "P");
        assert_eq!(site.name, "Imagine One");
        assert_eq!(site.logo_url, None);
        assert_eq!(site.bar_background, "#f5f6f8");
        assert_eq!(absolutize(base, Some(&json!("data:image/png;base64,AAA"))).as_deref(), Some("data:image/png;base64,AAA"));
    }

    #[test]
    fn packaged_config_and_electron_file() {
        let raw = r#"{ "baseUrl": "nefco.imagineone.it", "tenantSlug": "NEFCO" }"#.to_string();
        let p: Packaged = serde_json::from_str(&raw).unwrap();
        let packaged = Some((raw, p));
        assert_eq!(Config::packaged_slug(&packaged), "nefco");
        let dir = std::env::temp_dir().join(format!("i1frame-test-{}", std::process::id()));
        // The Electron app's file reads back, extra fields kept.
        let stored: Stored = serde_json::from_str(r#"{"baseUrl":"https://old.example","tenantSlug":"","seeded":true,"seededStamp":"x","extra":1}"#).unwrap();
        let mut c = Config { dir: dir.clone(), packaged, stored, dev_server: None };
        c.apply_packaged_defaults();
        assert_eq!(c.get(), FrameConfig { base_url: "https://nefco.imagineone.it".into(), tenant_slug: "nefco".into() });
        // The user picks another org; it stays until the packaged file changes.
        c.set(None, Some("other"));
        c.apply_packaged_defaults();
        assert_eq!(c.get().tenant_slug, "other");
        let back: Value = serde_json::from_str(&fs::read_to_string(c.file()).unwrap()).unwrap();
        assert_eq!(back["extra"], 1);
        assert_eq!(back["baseUrl"], "https://nefco.imagineone.it");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn page_script() {
        let js = app_js("0.2.0");
        assert!(js.contains("version: \"0.2.0\""));
        assert!(!js.contains("__VERSION__"));
        assert_eq!(clean_slug(" Acme-Corp!! "), "acme-corp");
    }
}
