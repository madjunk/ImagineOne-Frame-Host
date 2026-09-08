import { useEffect, useRef } from 'react'
import type { FrameState } from '../../../shared/types'

interface Props {
  state: FrameState
}

/**
 * Title bar.
 *
 * With the app's sidebar on screen the bar is two-tone: a block exactly as
 * wide (and as coloured) as the sidebar — holding ☰ and the logo — so the
 * sidebar visually continues up into the window chrome and follows it when it
 * expands/collapses; the rest takes the page background and holds the org
 * name, user and settings. Without a sidebar (sign-in, org step) the whole bar
 * wears the resolved branding colour.
 */
export function TitleBar({ state }: Props) {
  const { branding, user, online, platform, phase, sidebar } = state
  const menuBtn = useRef<HTMLButtonElement>(null)

  const openMenu = () => {
    const r = menuBtn.current?.getBoundingClientRect()
    window.frameHost.openMenu(r ? r.left : 8, r ? r.bottom : 40)
  }

  const split = !!sidebar && phase === 'ready' && !branding.barExplicit
  const blockWidth = split ? sidebar!.left + sidebar!.width : 0
  const blockBg = split && sidebar!.color ? sidebar!.color : branding.barBackground
  const blockFg = split && sidebar!.color ? contrastText(sidebar!.color) : branding.barForeground
  const restBg = split ? sidebar!.pageColor || '#ffffff' : branding.barBackground
  const restFg = split ? contrastText(restBg) : branding.barForeground
  const showLogoInBlock = !split || blockWidth >= 150

  // The OS paints the minimize/maximize/close strip: keep it the exact colour
  // of the bar's right side.
  useEffect(() => {
    window.frameHost.reportBarColors(restBg, restFg)
  }, [restBg, restFg])

  return (
    <header className={`titlebar titlebar--${platform}${split ? ' titlebar--split' : ''}`} style={{ background: restBg, color: restFg }}>
      <div
        className="titlebar__block"
        style={split ? { width: blockWidth, background: blockBg, color: blockFg } : { background: 'transparent', color: 'inherit' }}
      >
        <button ref={menuBtn} className="titlebar__menu" title="Menu" onClick={openMenu} aria-label="Menu">
          <Icon d="M4 6h16M4 12h16M4 18h16" />
        </button>
        {showLogoInBlock && <Logo branding={branding} />}
      </div>

      <div className="titlebar__brand" title={branding.name}>
        {!showLogoInBlock && <Logo branding={branding} />}
        {/* With a logo the name is already on screen; show text only without one. */}
        {!branding.logoUrl && <span className="titlebar__title">{branding.name}</span>}
      </div>

      <div className="titlebar__right">
        {!online && <span className="pill pill--warn">Offline</span>}
        {phase === 'ready' && user && (
          <span className="titlebar__user" title={user.email}>
            {user.name || user.email}
          </span>
        )}
        {canOpenSettings(state) && (
        <button title="Frame settings" onClick={() => window.frameHost.openSettings()} aria-label="Frame settings">
          <Icon d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
        </button>
        )}
      </div>
    </header>
  )
}

/** Frame settings (server URL, organisation) are for signed-in admins only. */
function canOpenSettings(state: FrameState): boolean {
  return !!state.user && (state.user.role === 'admin' || state.user.role === 'super_admin')
}

function Logo({ branding }: { branding: FrameState['branding'] }) {
  return branding.logoUrl ? (
    <img src={branding.logoUrl} alt="" className="titlebar__logo" />
  ) : (
    <span className="titlebar__tile" style={{ background: branding.primaryColor }}>
      {initials(branding.name)}
    </span>
  )
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean)
  const s = parts.length >= 2 ? parts[0][0] + parts[1][0] : name.slice(0, 2)
  return s.toUpperCase()
}

/** Black or white text for a CSS colour (hex or rgb[a]). */
function contrastText(color: string): string {
  let r = 0, g = 0, b = 0
  const hex = color.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i)
  const rgb = color.match(/rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/i)
  if (hex) {
    let h = hex[1]
    if (h.length === 3) h = h.split('').map((c) => c + c).join('')
    r = parseInt(h.slice(0, 2), 16); g = parseInt(h.slice(2, 4), 16); b = parseInt(h.slice(4, 6), 16)
  } else if (rgb) {
    r = +rgb[1]; g = +rgb[2]; b = +rgb[3]
  } else return '#1f2328'
  const lin = (c: number) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4 }
  const L = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
  return L > 0.5 ? '#1f2328' : '#ffffff'
}

export function Icon({ d, size = 18 }: { d: string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  )
}
