import { DEFAULT_BRANDING, type FrameBranding, type TenantBranding } from '../shared/types'
import { fetchSettings, fetchSiteSettings, fetchTenant } from './api'

/** The web app's default sidebar background (hsl(199,100%,22%)). */
const APP_DEFAULT_SIDEBAR = '#004d70'

/** Absolute URL for a logo that may be stored as a relative path. */
function absolutize(baseUrl: string, url: string | null | undefined): string | null {
  if (!url) return null
  try {
    return new URL(url, baseUrl + '/').toString()
  } catch {
    return null
  }
}

function isHex(c: unknown): c is string {
  return typeof c === 'string' && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(c.trim())
}

function expandHex(c: string): string {
  const h = c.trim().replace('#', '')
  return '#' + (h.length === 3 ? h.split('').map((x) => x + x).join('') : h)
}

/** WCAG-ish relative luminance → pick black or white text for a background. */
export function contrastForeground(bg: string): string {
  const h = expandHex(bg).slice(1)
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
  const L = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
  return L > 0.5 ? '#1f2328' : '#ffffff'
}

function fromTenant(baseUrl: string, t: TenantBranding): FrameBranding {
  const bar = isHex(t.primaryColor) ? expandHex(t.primaryColor) : DEFAULT_BRANDING.barBackground
  return {
    source: 'tenant',
    name: t.appTitle || t.name,
    logoUrl: absolutize(baseUrl, t.logoUrl),
    barBackground: bar,
    barForeground: contrastForeground(bar),
    primaryColor: isHex(t.accentColor) ? expandHex(t.accentColor) : bar,
    barExplicit: false,
    logoBackground: null
  }
}

function fromSettings(baseUrl: string, s: Record<string, string | null>): FrameBranding | null {
  const name = s['company-name'] || s['app-title'] || ''
  const logo = s['company-logo-url'] || null
  // The app's sidebar colour continues into the frame's title bar, so the
  // window reads as one surface. When the tenant has not customised it, the
  // sidebar renders the app's built-in navy (client/src/index.css
  // --sidebar-background: hsl(199,100%,22%)), so match that rather than the
  // lighter title/logo strip.
  const titleBg = s['theme-sidebar-color'] || APP_DEFAULT_SIDEBAR
  const primary = s['theme-primary-color']
  const text = s['theme-text-color']
  // Independent title bar colours (Theme Customization → Desktop Title Bar).
  // When set, the bar is one solid colour and stops following the sidebar.
  const explicitBar = s['theme-titlebar-background-color']
  const explicitText = s['theme-titlebar-text-color']
  if (!name && !logo && !isHex(titleBg) && !isHex(primary) && !isHex(explicitBar)) return null

  const explicit = isHex(explicitBar)
  const bar = explicit
    ? expandHex(explicitBar)
    : isHex(titleBg) ? expandHex(titleBg) : isHex(primary) ? expandHex(primary) : DEFAULT_BRANDING.barBackground
  const fg = explicit
    ? (isHex(explicitText) ? expandHex(explicitText) : contrastForeground(bar))
    : (isHex(text) ? expandHex(text) : contrastForeground(bar))
  return {
    source: 'settings',
    name: name || DEFAULT_BRANDING.name,
    logoUrl: absolutize(baseUrl, logo),
    barBackground: bar,
    barForeground: fg,
    primaryColor: isHex(primary) ? expandHex(primary) : bar,
    barExplicit: explicit,
    logoBackground: isHex(s['theme-logo-background-color']) ? expandHex(s['theme-logo-background-color'] as string) : null
  }
}

function fromSite(baseUrl: string, s: Record<string, unknown>): FrameBranding {
  const name = (s['siteName'] as string) || (s['companyName'] as string) || (s['appTitle'] as string) || DEFAULT_BRANDING.name
  const primary = s['primaryColor'] ?? s['themePrimaryColor']
  const bar = isHex(primary) ? expandHex(primary) : DEFAULT_BRANDING.barBackground
  return {
    source: 'site',
    name,
    logoUrl: absolutize(baseUrl, s['logoUrl'] as string | null),
    barBackground: bar,
    barForeground: contrastForeground(bar),
    primaryColor: isHex(primary) ? expandHex(primary) : DEFAULT_BRANDING.primaryColor,
    barExplicit: false,
    logoBackground: null
  }
}

/** Slug from a /t/:slug login route, if the app view is on one. */
export function slugFromUrl(url: string): string | null {
  try {
    const m = new URL(url).pathname.match(/^\/t\/([^/]+)/)
    return m ? decodeURIComponent(m[1]).toLowerCase() : null
  } catch {
    return null
  }
}

export interface BrandingContext {
  baseUrl: string
  configuredSlug: string
  currentUrl: string
  signedIn: boolean
}

/**
 * Resolve what the title bar should show right now. Never throws — falls
 * back down the chain and finally to DEFAULT_BRANDING.
 */
export async function resolveBranding(ctx: BrandingContext): Promise<FrameBranding> {
  const { baseUrl } = ctx

  const slug = ctx.configuredSlug || slugFromUrl(ctx.currentUrl)

  if (ctx.signedIn) {
    const [settings, tenant] = await Promise.all([
      fetchSettings(baseUrl).catch(() => null),
      slug ? fetchTenant(baseUrl, slug).catch(() => null) : Promise.resolve(null)
    ])
    const b = settings ? fromSettings(baseUrl, settings) : null
    if (b) {
      // /api/settings falls back to platform-level values for keys the tenant
      // never set (company-logo-url, company-name…), which would paint the
      // mother's logo on a customer's window. The tenant's own public
      // branding is authoritative for identity; settings still supply colours.
      if (tenant) {
        b.logoUrl = absolutize(baseUrl, tenant.logoUrl) ?? b.logoUrl
        b.name = tenant.appTitle || tenant.name || b.name
      }
      return b
    }
    if (tenant) return fromTenant(baseUrl, tenant)
  } else if (slug) {
    const t = await fetchTenant(baseUrl, slug).catch(() => null)
    if (t) return fromTenant(baseUrl, t)
  }

  const site = await fetchSiteSettings(baseUrl, slug ?? undefined).catch(() => null)
  if (site) return fromSite(baseUrl, site)

  return DEFAULT_BRANDING
}
