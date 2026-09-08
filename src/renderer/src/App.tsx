import { useEffect, useState } from 'react'
import type { FrameState } from '../../shared/types'
import { TitleBar } from './components/TitleBar'
import { StatusScreen } from './components/StatusScreen'
import { SettingsPanel } from './components/SettingsPanel'
import { OrgScreen } from './components/OrgScreen'

/**
 * The renderer only draws the frame chrome: a 40px title bar plus full-window
 * overlays (boot status, config error, settings). The hosted web app lives in a
 * WebContentsView positioned below the title bar by the main process, so
 * nothing here is layered on top of it while it's visible.
 */
export default function App() {
  const [state, setState] = useState<FrameState | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)

  useEffect(() => {
    void window.frameHost.getState().then(setState)
    const offState = window.frameHost.onState(setState)
    const offSettings = window.frameHost.onOpenSettings(() => setSettingsOpen(true))
    const report = () => window.frameHost.reportOnline(navigator.onLine)
    window.addEventListener('online', report)
    window.addEventListener('offline', report)
    report()
    return () => {
      offState()
      offSettings()
      window.removeEventListener('online', report)
      window.removeEventListener('offline', report)
    }
  }, [])

  useEffect(() => {
    if (!state) return
    document.documentElement.style.setProperty('--brand', state.branding.primaryColor)
  }, [state?.branding.primaryColor])

  if (!state) return null

  const closeSettings = () => {
    setSettingsOpen(false)
    window.frameHost.settingsClosed()
  }

  return (
    <div className="frame">
      <TitleBar state={state} />
      <main className="frame-body">
        {settingsOpen ? (
          <SettingsPanel
            config={state.config}
            errorMessage={state.errorMessage}
            canCancel={state.phase !== 'config-error'}
            onSave={(next) => {
              setSettingsOpen(false)
              window.frameHost.saveSettings(next)
            }}
            onCancel={closeSettings}
          />
        ) : state.phase === 'ready' || state.phase === 'login' ? null : state.phase === 'org' ? (
          <OrgScreen state={state} />
        ) : (
          <StatusScreen state={state} onOpenSettings={() => setSettingsOpen(true)} />
        )}
      </main>
    </div>
  )
}
