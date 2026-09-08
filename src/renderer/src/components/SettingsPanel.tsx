import { useState } from 'react'
import type { FrameConfig } from '../../../shared/types'

interface Props {
  config: FrameConfig
  errorMessage: string | null
  canCancel: boolean
  onSave: (next: Partial<FrameConfig>) => void
  onCancel: () => void
}

export function SettingsPanel({ config, errorMessage, canCancel, onSave, onCancel }: Props) {
  const [baseUrl, setBaseUrl] = useState(config.baseUrl)
  const [tenantSlug, setTenantSlug] = useState(config.tenantSlug)

  return (
    <div className="status">
      <form
        className="settings"
        onSubmit={(e) => {
          e.preventDefault()
          onSave({ baseUrl, tenantSlug })
        }}
      >
        <h1>Frame settings</h1>
        {errorMessage && <p className="settings__error">{errorMessage}</p>}

        <label>
          Server URL
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://app.imagineone.it"
            autoFocus
            spellCheck={false}
          />
          <small>The domain this install is deployed on. Only this domain may load inside the frame.</small>
        </label>

        <label>
          Company slug <span className="muted">(optional)</span>
          <input
            type="text"
            value={tenantSlug}
            onChange={(e) => setTenantSlug(e.target.value)}
            placeholder="acme"
            spellCheck={false}
          />
          <small>Locks this install to one company. Verified against the server at every start.</small>
        </label>

        <div className="status__actions">
          <button className="btn btn--primary" type="submit">
            Save &amp; reconnect
          </button>
          {canCancel && (
            <button className="btn" type="button" onClick={onCancel}>
              Cancel
            </button>
          )}
        </div>
      </form>
    </div>
  )
}
