import { useEffect, useState } from 'react'
import type { FrameState } from '../../../shared/types'

interface Props {
  state: FrameState
}

/**
 * Compact-window first step: which organisation? The slug is verified by the
 * main process against /api/public/tenant/:slug before the app's own sign-in
 * page is shown.
 */
/**
 * The Continue button takes the brand colour, but a very light primary (or
 * white) would vanish on the pale page — fall back to the bar colour, then to
 * a solid blue, and always pick readable text.
 */
function buttonStyle(b: FrameState['branding']): { background: string; color: string } {
  const candidates = [b.primaryColor, b.barBackground, '#1d4ed8']
  const bg = candidates.find((c) => luminance(c) < 0.6) ?? '#1d4ed8'
  return { background: bg, color: luminance(bg) > 0.5 ? '#111827' : '#ffffff' }
}

function luminance(color: string): number {
  const m = color.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i)
  if (!m) return 0
  let h = m[1]
  if (h.length === 3) h = h.split('').map((c) => c + c).join('')
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

export function OrgScreen({ state }: Props) {
  const { branding, orgError, orgBusy, config } = state
  const [slug, setSlug] = useState(config.tenantSlug)

  useEffect(() => setSlug(config.tenantSlug), [config.tenantSlug])

  return (
    <div className="org">
      <form
        className="org__card"
        onSubmit={(e) => {
          e.preventDefault()
          if (!orgBusy) window.frameHost.continueWithOrg(slug)
        }}
      >
        {branding.logoUrl ? (
          <img src={branding.logoUrl} alt="" className="org__logo" />
        ) : (
          <div className="org__tile" style={{ background: branding.primaryColor }}>
            {branding.name.slice(0, 2).toUpperCase()}
          </div>
        )}
        <div className="org__brand">{branding.name}</div>

        <h1>Sign in</h1>
        <p className="org__lead">Enter your organisation's identifier to continue</p>

        <label className="org__label" htmlFor="org-slug">
          Organisation
        </label>
        <input
          id="org-slug"
          className={`org__input${orgError ? ' org__input--error' : ''}`}
          value={slug}
          onChange={(e) => setSlug(e.target.value)}
          placeholder="e.g. acme-corp"
          autoFocus
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          disabled={orgBusy}
        />
        <small className={orgError ? 'org__hint org__hint--error' : 'org__hint'}>
          {orgError ?? 'Use the identifier provided by your administrator.'}
        </small>

        <button className="org__button" type="submit" disabled={orgBusy} style={buttonStyle(branding)}>
          {orgBusy ? 'Checking…' : 'Continue'}
        </button>

        <div className="org__footer">© {new Date().getFullYear()} Imagine One IT</div>
      </form>
    </div>
  )
}
