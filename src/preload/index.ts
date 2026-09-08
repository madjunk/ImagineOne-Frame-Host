import { contextBridge, ipcRenderer } from 'electron'
import type { FrameState, FrameConfig } from '../shared/types'

/**
 * The only bridge between the renderer title bar and the main process.
 * The hosted web app (in the WebContentsView) never sees this — it has no preload.
 */
const api = {
  getState: (): Promise<FrameState> => ipcRenderer.invoke('frame:get-state'),
  onState: (cb: (s: FrameState) => void): (() => void) => {
    const handler = (_e: unknown, s: FrameState) => cb(s)
    ipcRenderer.on('frame:state', handler)
    return () => ipcRenderer.removeListener('frame:state', handler)
  },
  onOpenSettings: (cb: () => void): (() => void) => {
    const handler = () => cb()
    ipcRenderer.on('frame:open-settings', handler)
    return () => ipcRenderer.removeListener('frame:open-settings', handler)
  },
  openMenu: (x: number, y: number) => ipcRenderer.send('frame:open-menu', x, y),
  home: () => ipcRenderer.send('frame:action', 'home'),
  reload: () => ipcRenderer.send('frame:action', 'reload'),
  back: () => ipcRenderer.send('frame:action', 'back'),
  forward: () => ipcRenderer.send('frame:action', 'forward'),
  signOut: () => ipcRenderer.send('frame:action', 'signOut'),
  continueWithOrg: (slug: string) => ipcRenderer.send('frame:action', 'continueWithOrg', slug),
  changeOrg: () => ipcRenderer.send('frame:action', 'changeOrg'),
  openSettings: () => ipcRenderer.send('frame:action', 'openSettings'),
  saveSettings: (next: Partial<FrameConfig>) => ipcRenderer.send('frame:action', 'saveSettings', next),
  settingsClosed: () => ipcRenderer.send('frame:settings-closed'),
  checkForUpdates: () => ipcRenderer.send('frame:action', 'checkForUpdates'),
  reportOnline: (online: boolean) => ipcRenderer.send('frame:online', online),
  reportBarColors: (background: string, foreground: string) => ipcRenderer.send('frame:bar-colors', background, foreground)
}

export type FrameHostApi = typeof api

contextBridge.exposeInMainWorld('frameHost', api)
