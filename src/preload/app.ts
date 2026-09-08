import { contextBridge, ipcRenderer } from 'electron'

/**
 * Preload for the hosted web app (sandboxed, context-isolated).
 *
 * 1. Sidebar watcher — *measures* where the app's sidebar is, how wide it is
 *    and what colours the sidebar and page use, so the frame's title bar can
 *    continue the sidebar upward and follow it when it expands/collapses.
 * 2. A narrow `window.imagineOneFrame` bridge the app's sign-in page uses for
 *    "Remember me on this device": credentials are stored by the main process
 *    encrypted with the OS user account, never in web storage.
 */

const VERSION = process.env['npm_package_version'] ?? ''

contextBridge.exposeInMainWorld('imagineOneFrame', {
  version: VERSION,
  credentials: {
    get: (slug: string) => ipcRenderer.invoke('frame:credentials:get', String(slug)),
    save: (slug: string, creds: { email: string; password: string }) =>
      ipcRenderer.invoke('frame:credentials:save', String(slug), { email: String(creds?.email ?? ''), password: String(creds?.password ?? '') }),
    clear: (slug: string) => ipcRenderer.invoke('frame:credentials:clear', String(slug))
  }
})
interface SidebarMetrics {
  left: number
  width: number
  color: string
  pageColor: string
}

// The app's main sidebar (components/sidebar.tsx); the ui/sidebar primitive is a fallback.
const SIDEBAR = 'aside[data-testid="sidebar"], [data-sidebar="sidebar"]'
let last = ''
let raf = 0

function opaque(color: string): boolean {
  return !!color && color !== 'transparent' && !/rgba\(\s*\d+,\s*\d+,\s*\d+,\s*0\)/.test(color)
}

/** Walk up from `el` to find the first non-transparent background. */
function backgroundOf(el: Element | null): string {
  for (let n: Element | null = el; n; n = n.parentElement) {
    const bg = getComputedStyle(n).backgroundColor
    if (opaque(bg)) return bg
  }
  return ''
}

function measure(): void {
  raf = 0
  const el = document.querySelector<HTMLElement>(SIDEBAR)
  let payload: SidebarMetrics | null = null
  if (el) {
    const r = el.getBoundingClientRect()
    // Hidden (mobile layout) or off-canvas sidebars report a zero box.
    if (r.width > 0 && r.height > 0 && r.right > 0) {
      payload = {
        left: Math.round(r.left),
        width: Math.round(r.width),
        color: backgroundOf(el),
        pageColor: backgroundOf(document.body) || getComputedStyle(document.documentElement).backgroundColor
      }
    }
  }
  const key = JSON.stringify(payload)
  if (key !== last) {
    last = key
    ipcRenderer.send('frame:sidebar', payload)
  }
}

function schedule(): void {
  if (!raf) raf = requestAnimationFrame(measure)
}

function start(): void {
  // Structure changes (route changes mount/unmount the sidebar), size changes
  // (expand/collapse transition), and window resizes.
  new MutationObserver(schedule).observe(document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['class', 'style', 'data-state', 'data-collapsible']
  })
  new ResizeObserver(schedule).observe(document.documentElement)
  window.addEventListener('resize', schedule)
  window.addEventListener('transitionend', schedule, true)
  // Colour changes from the theme applicator arrive without a resize.
  setInterval(schedule, 1500)
  schedule()
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start)
else start()
