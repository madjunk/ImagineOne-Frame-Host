import { app } from 'electron'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'fs'
import { dirname, join } from 'path'
import type { FrameConfig } from '../shared/types'

/**
 * Per-install configuration, persisted as plain JSON in the user's app-data
 * folder (Windows: %APPDATA%\ImagineOne\frame-config.json,
 * macOS: ~/Library/Application Support/ImagineOne/frame-config.json).
 *
 * Defaults can be overridden at packaging time by shipping a
 * `frame-config.json` in the app's resources folder — used to produce a
 * customer-specific installer locked to one domain + slug. It is applied once,
 * on first run; afterwards the user-level file wins.
 */
const DEFAULTS: FrameConfig = {
  baseUrl: 'https://app.imagineone.it',
  tenantSlug: ''
}

interface StoredConfig extends FrameConfig {
  seeded?: boolean
}

let cache: StoredConfig | null = null

function filePath(): string {
  return join(app.getPath('userData'), 'frame-config.json')
}

function load(): StoredConfig {
  if (cache) return cache
  try {
    const raw = JSON.parse(readFileSync(filePath(), 'utf8')) as Partial<StoredConfig>
    cache = { ...DEFAULTS, ...raw }
  } catch {
    cache = { ...DEFAULTS }
  }
  return cache
}

function save(next: StoredConfig): void {
  cache = next
  const p = filePath()
  mkdirSync(dirname(p), { recursive: true })
  writeFileSync(p, JSON.stringify(next, null, 2))
}

export function getConfig(): FrameConfig {
  const { baseUrl, tenantSlug } = load()
  return { baseUrl: normalizeBaseUrl(baseUrl), tenantSlug }
}

export function setConfig(next: Partial<FrameConfig>): FrameConfig {
  const cur = load()
  save({
    ...cur,
    baseUrl: next.baseUrl !== undefined ? normalizeBaseUrl(next.baseUrl) : cur.baseUrl,
    tenantSlug: next.tenantSlug !== undefined ? next.tenantSlug.trim().toLowerCase() : cur.tenantSlug
  })
  return getConfig()
}

/**
 * Reduce whatever was typed/pasted ("app.imagineone.it", a full page URL with
 * a path, trailing slashes…) to the bare origin, e.g. https://app.imagineone.it.
 */
export function normalizeBaseUrl(input: string): string {
  let url = (input || '').trim()
  if (!url) return DEFAULTS.baseUrl
  if (!/^https?:\/\//i.test(url)) url = `https://${url}`
  try {
    return new URL(url).origin
  } catch {
    return DEFAULTS.baseUrl
  }
}

/** Apply a packaged `frame-config.json` once, on first run, if present. */
export function applyPackagedDefaults(candidatePath: string): void {
  const cur = load()
  if (cur.seeded) return
  let packaged: Partial<FrameConfig> = {}
  try {
    if (existsSync(candidatePath)) packaged = JSON.parse(readFileSync(candidatePath, 'utf8'))
  } catch {
    // malformed packaged config — ignore; user can fix it in Frame settings
  }
  setConfig(packaged)
  save({ ...load(), seeded: true })
}
