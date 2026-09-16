import { app } from 'electron'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'fs'
import { createHash } from 'crypto'
import { dirname, join } from 'path'
import type { FrameConfig } from '../shared/types'

/**
 * Per-install configuration, persisted as plain JSON in the user's app-data
 * folder. That folder is made **per customer** (keyed by the packaged tenant
 * slug — see the isolation block at the top of main/index.ts), so two customer
 * installs on the same machine never share config, cache or cookies.
 *
 * Defaults can be overridden at packaging time by shipping a
 * `frame-config.json` in the app's resources folder — used to produce a
 * customer-specific installer locked to one domain + slug. It is (re)applied
 * whenever the packaged file's contents change (see `applyPackagedDefaults`),
 * so re-issuing a corrected installer actually updates an install that already
 * ran, instead of being silently ignored. A user's own later edits in Frame
 * settings survive as long as the packaged file itself is unchanged.
 */
const DEFAULTS: FrameConfig = {
  baseUrl: 'https://app.imagineone.it',
  tenantSlug: ''
}

interface StoredConfig extends FrameConfig {
  seeded?: boolean
  /** Hash of the packaged frame-config.json last applied; re-applied on change. */
  seededStamp?: string
}

let cache: StoredConfig | null = null

function filePath(): string {
  return join(app.getPath('userData'), 'frame-config.json')
}

/**
 * Where a packaged (customer-locked) `frame-config.json` lives: the app's
 * resources folder in a built app, the project root in dev.
 */
export function packagedConfigPath(): string {
  return join(app.isPackaged ? process.resourcesPath : process.cwd(), 'frame-config.json')
}

function readPackaged(): { raw: string; config: Partial<FrameConfig> } | null {
  try {
    const p = packagedConfigPath()
    if (!existsSync(p)) return null
    const raw = readFileSync(p, 'utf8')
    return { raw, config: JSON.parse(raw) as Partial<FrameConfig> }
  } catch {
    return null
  }
}

/**
 * The tenant slug baked into the packaged installer, if any. Read straight from
 * the packaged file (not the user-level config) so it is available *before*
 * `app` is ready — used to key the per-customer data folder (main/index.ts) and
 * cookie partition (main/api.ts). Empty string for the unlocked/main build, or
 * when no packaged config ships.
 */
export function packagedTenantSlug(): string {
  const packaged = readPackaged()
  return (packaged?.config.tenantSlug || '').trim().toLowerCase()
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

/**
 * Apply the packaged `frame-config.json` defaults. Runs on first launch, and
 * again whenever the packaged file's contents change — so shipping a corrected
 * installer to a machine that already ran the app actually updates its
 * baseUrl/slug instead of being ignored (the previous "apply once" behaviour
 * silently kept the stale URL forever). When the packaged file is unchanged,
 * the user-level config — including any edits made in Frame settings — is left
 * untouched.
 */
export function applyPackagedDefaults(candidatePath: string = packagedConfigPath()): void {
  let raw = ''
  let packaged: Partial<FrameConfig> = {}
  try {
    if (!existsSync(candidatePath)) return
    raw = readFileSync(candidatePath, 'utf8')
    packaged = JSON.parse(raw) as Partial<FrameConfig>
  } catch {
    // no packaged config, or malformed — leave the user config as-is; it can be
    // fixed from the Frame settings panel.
    return
  }
  const stamp = createHash('sha1').update(raw).digest('hex')
  const cur = load()
  if (cur.seeded && cur.seededStamp === stamp) return
  setConfig(packaged)
  save({ ...load(), seeded: true, seededStamp: stamp })
}
