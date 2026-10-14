/*
 * window.frameHost for the Tauri build (tauri/). The Electron build gets the same API from
 * src/preload/index.ts; App.tsx works with either. The Rust side (tauri/src-tauri/src/lib.rs)
 * accepts these commands only from this title-bar page, pushes every state change by calling
 * window.__frameState(state), and opens the settings panel with window.__frameOpenSettings().
 */
import type { FrameConfig, FrameState } from '../../shared/types'

type Internals = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }
const w = window as unknown as {
  __TAURI_INTERNALS__?: Internals
  frameHost?: unknown
  __frameState?: (s: FrameState) => void
  __frameOpenSettings?: () => void
}
const tauri = w.__TAURI_INTERNALS__

if (tauri && !w.frameHost) {
  const call = (cmd: string, args?: Record<string, unknown>) => tauri.invoke(cmd, args ?? {})
  const fire = (cmd: string, args?: Record<string, unknown>) => {
    call(cmd, args).catch(() => undefined)
  }
  const stateListeners = new Set<(s: FrameState) => void>()
  const settingsListeners = new Set<() => void>()
  w.__frameState = (s) => stateListeners.forEach((cb) => cb(s))
  w.__frameOpenSettings = () => settingsListeners.forEach((cb) => cb())
  const action = (name: string) => fire('frame_action', { name })

  w.frameHost = {
    getState: () => call('frame_get_state') as Promise<FrameState>,
    onState: (cb: (s: FrameState) => void) => {
      stateListeners.add(cb)
      return () => {
        stateListeners.delete(cb)
      }
    },
    onOpenSettings: (cb: () => void) => {
      settingsListeners.add(cb)
      return () => {
        settingsListeners.delete(cb)
      }
    },
    openMenu: (x: number, y: number) => fire('frame_open_menu', { x, y }),
    home: () => action('home'),
    reload: () => action('reload'),
    back: () => action('back'),
    forward: () => action('forward'),
    signOut: () => action('signOut'),
    continueWithOrg: (slug: string) => fire('frame_continue_org', { slug: String(slug) }),
    changeOrg: () => action('changeOrg'),
    openSettings: () => action('openSettings'),
    // Retry on the "configuration problem" screen sends no changes: start again.
    saveSettings: (next: Partial<FrameConfig>) =>
      next && (next.baseUrl !== undefined || next.tenantSlug !== undefined)
        ? fire('frame_save_settings', { baseUrl: next.baseUrl, tenantSlug: next.tenantSlug })
        : action('retry'),
    settingsClosed: () => fire('frame_settings_closed'),
    checkForUpdates: () => action('checkForUpdates'),
    reportOnline: (online: boolean) => fire('frame_online', { online: !!online }),
    minimize: () => fire('frame_win', { what: 'minimize' }),
    toggleMaximize: () => fire('frame_win', { what: 'toggleMaximize' }),
    closeWindow: () => fire('frame_win', { what: 'close' })
  }
}

export {}
