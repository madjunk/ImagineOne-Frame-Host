import type { FrameState } from '../../../shared/types'

interface Props {
  state: FrameState
  onOpenSettings: () => void
}

export function StatusScreen({ state, onOpenSettings }: Props) {
  if (state.phase === 'starting') {
    return (
      <div className="status">
        <div className="spinner" />
        <p>Connecting to {state.config.baseUrl}…</p>
      </div>
    )
  }

  return (
    <div className="status status--error">
      <h1>Configuration problem</h1>
      <p className="status__message">{state.errorMessage}</p>
      <p className="status__hint">
        Check that this computer can reach the server and that the company slug matches the install.
      </p>
      <div className="status__actions">
        <button className="btn btn--primary" onClick={onOpenSettings}>
          Open frame settings
        </button>
        <button className="btn" onClick={() => window.frameHost.saveSettings({})}>
          Retry
        </button>
      </div>
    </div>
  )
}
