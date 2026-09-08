// Types shared between main, preload and renderer.

export interface FrameConfig {
  /** Base URL of this install, e.g. https://app.imagineone.it (one domain per customer install). */
  baseUrl: string
  /** Company slug this install is locked to. Empty = no tenant lock (main/mother instance). */
  tenantSlug: string
}

export interface TenantBranding {
  id: string
  name: string
  slug: string
  appTitle: string
  logoUrl: string
  faviconUrl: string
  primaryColor: string
  accentColor: string
  publicRegistrationEnabled: boolean
  authProviders: { local: boolean; azure: boolean; google: boolean }
}

export interface CurrentUser {
  id: string
  email: string
  name: string
  role: 'super_admin' | 'admin' | 'project_manager' | 'project_user' | 'user' | 'form_access'
  tenantId: string
  isDemo: boolean
  instanceRole: 'main' | 'child' | string
  selfHostedConsoleUrl: string
}

/**
 * What the title bar actually paints. Resolved by the main process from
 * whichever source applies right now:
 *  - 'settings'  GET /api/settings (signed in; tenant-scoped theme + company)
 *  - 'tenant'    GET /api/public/tenant/:slug (login page for a company)
 *  - 'site'      GET /api/public/site-settings (anything else, pre-login)
 *  - 'default'   nothing reachable yet
 */
export interface FrameBranding {
  source: 'settings' | 'tenant' | 'site' | 'default'
  name: string
  logoUrl: string | null
  /** Title bar background. */
  barBackground: string
  /** Title bar text/icon color (chosen for contrast with barBackground). */
  barForeground: string
  /** Accent for pills/spinner. */
  primaryColor: string
  /** True when the tenant set explicit title bar colours: the bar is one solid colour and does not follow the sidebar. */
  barExplicit: boolean
  /** Theme "Logo Background" — the strip at the top of the sidebar; the split bar's left block continues it. */
  logoBackground: string | null
}

export type FramePhase =
  | 'starting' // app booted, checking reachability
  | 'config-error' // unreachable or slug mismatch — show settings
  | 'org' // compact window: frame-drawn organisation step
  | 'login' // compact window: the app's own /t/:slug login page
  | 'ready' // full window: app view visible

/** Window geometry for the compact sign-in window. */
export const COMPACT_WINDOW = { width: 480, height: 700 }

/** Live geometry of the hosted app's sidebar, reported by the app-view watcher. */
export interface SidebarMetrics {
  left: number
  width: number
  color: string
  pageColor: string
}

export interface FrameState {
  phase: FramePhase
  /** null when no sidebar is on screen (login pages, org step). */
  sidebar: SidebarMetrics | null
  config: FrameConfig
  branding: FrameBranding
  user: CurrentUser | null
  errorMessage: string | null
  /** Inline error for the organisation step (unknown slug etc.). */
  orgError: string | null
  orgBusy: boolean
  online: boolean
  canGoBack: boolean
  canGoForward: boolean
  appVersion: string
  platform: NodeJS.Platform
}

/** Height of the renderer-drawn title bar; the app view sits below it. */
export const TITLE_BAR_HEIGHT = 48

export const DEFAULT_BRANDING: FrameBranding = {
  source: 'default',
  name: 'Imagine One IT',
  logoUrl: null,
  barBackground: '#f5f6f8',
  barForeground: '#1f2328',
  primaryColor: '#2563eb',
  barExplicit: false,
  logoBackground: null
}
