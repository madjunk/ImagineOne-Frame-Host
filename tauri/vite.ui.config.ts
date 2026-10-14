// The title bar and its screens (src/renderer) built for the Tauri app: npm run build:ui -> tauri/ui-dist.
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import { resolve } from 'path'

// The page's CSP must also let it reach Tauri's IPC (ipc: / http://ipc.localhost).
const tauriCsp = (): Plugin => ({
  name: 'tauri-csp',
  transformIndexHtml: (html) => html.replace("default-src 'self';", "default-src 'self'; connect-src 'self' ipc: http://ipc.localhost;")
})

export default defineConfig({
  root: resolve(__dirname, '../src/renderer'),
  base: './',
  resolve: { alias: { '@': resolve(__dirname, '../src/renderer/src') } },
  plugins: [react(), tauriCsp()],
  build: { outDir: resolve(__dirname, 'ui-dist'), emptyOutDir: true }
})
