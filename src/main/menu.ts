import { app, Menu, type MenuItemConstructorOptions } from 'electron'
import type { FrameState } from '../shared/types'
import type { FrameActions } from './index'

/**
 * The single native menu (section 6 of frame-host-api-reference.md).
 * Rebuilt whenever state changes so the Administration section appears only
 * when /api/auth/me reports role === 'admin'.
 */
export function buildMenu(state: FrameState, a: FrameActions): Menu {
  const isMac = process.platform === 'darwin'
  const appName = state.branding.name
  const isAdmin = state.user?.role === 'admin' || state.user?.role === 'super_admin'
  // Frame settings (server URL / organisation): signed-in admins only.
  // Before sign-in, "Change organisation…" below covers the wrong-slug case.
  const canOpenSettings = isAdmin

  const adminSection: MenuItemConstructorOptions[] = isAdmin
    ? [
        {
          label: 'Administration',
          submenu: [
            { label: 'Settings', click: () => a.navigate('/admin/settings') },
            { label: 'Users', click: () => a.navigate('/admin/users') },
            { label: 'Software Update', click: () => a.navigate('/admin/software-update') },
            ...(state.user?.selfHostedConsoleUrl
              ? [
                  { type: 'separator' } as MenuItemConstructorOptions,
                  { label: 'Open deployment console', click: () => a.navigate(state.user!.selfHostedConsoleUrl) }
                ]
              : [])
          ]
        }
      ]
    : []

  const template: MenuItemConstructorOptions[] = [
    ...(isMac
      ? [
          {
            label: appName,
            submenu: [
              { label: `About ${appName}`, click: a.about },
              { label: 'Check for updates…', click: a.checkForUpdates },
              { type: 'separator' },
              ...(canOpenSettings ? [{ label: 'Frame settings…', accelerator: 'Cmd+,', click: a.openSettings }] : []),
              { type: 'separator' },
              { role: 'hide' },
              { role: 'hideOthers' },
              { role: 'unhide' },
              { type: 'separator' },
              { label: 'Quit', accelerator: 'Cmd+Q', click: a.quit }
            ]
          } as MenuItemConstructorOptions
        ]
      : []),
    {
      label: appName,
      submenu: [
        { label: 'Home', accelerator: 'Alt+Home', click: a.home },
        { label: 'Reload', accelerator: 'CmdOrCtrl+R', click: a.reload },
        { label: 'Back', accelerator: 'Alt+Left', enabled: state.canGoBack, click: a.back },
        { label: 'Forward', accelerator: 'Alt+Right', enabled: state.canGoForward, click: a.forward },
        { type: 'separator' },
        ...(state.user ? [{ label: `Sign out (${state.user.email})`, click: () => void a.signOut() }] : []),
        ...(state.config.tenantSlug && !state.user
          ? [{ label: `Change organisation… (${state.config.tenantSlug})`, click: () => void a.changeOrg() }]
          : []),
        ...(isMac
          ? []
          : [
              { type: 'separator' } as MenuItemConstructorOptions,
              ...(canOpenSettings ? [{ label: 'Frame settings…', click: a.openSettings }] : []),
              { label: 'Check for updates…', click: a.checkForUpdates },
              { label: 'About', click: a.about },
              { type: 'separator' } as MenuItemConstructorOptions,
              { label: 'Quit', accelerator: 'Ctrl+Q', click: a.quit }
            ])
      ]
    },
    ...adminSection,
    { role: 'editMenu' },
    {
      label: 'View',
      submenu: [
        { role: 'resetZoom' },
        { role: 'zoomIn' },
        { role: 'zoomOut' },
        { type: 'separator' },
        { role: 'togglefullscreen' },
        ...(!app.isPackaged
          ? [{ type: 'separator' } as MenuItemConstructorOptions, { role: 'toggleDevTools' } as MenuItemConstructorOptions]
          : [])
      ]
    },
    { role: 'windowMenu' }
  ]

  return Menu.buildFromTemplate(template)
}
