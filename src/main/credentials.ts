import { app, safeStorage } from 'electron'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'fs'
import { dirname, join } from 'path'

/**
 * "Remember me" store for the hosted app's sign-in form, per organisation.
 * Secrets are encrypted with the OS user account (Windows DPAPI / macOS
 * Keychain via Electron safeStorage) and kept in the user's app-data folder;
 * they never touch the web page's own storage.
 */
export interface StoredCredentials {
  email: string
  password: string
}

function filePath(): string {
  return join(app.getPath('userData'), 'credentials.json')
}

function readAll(): Record<string, string> {
  try {
    return JSON.parse(readFileSync(filePath(), 'utf8'))
  } catch {
    return {}
  }
}

function writeAll(data: Record<string, string>): void {
  const p = filePath()
  mkdirSync(dirname(p), { recursive: true })
  writeFileSync(p, JSON.stringify(data, null, 2))
}

export function credentialsAvailable(): boolean {
  return safeStorage.isEncryptionAvailable()
}

export function getCredentials(slug: string): StoredCredentials | null {
  if (!credentialsAvailable()) return null
  const blob = readAll()[slug.toLowerCase()]
  if (!blob) return null
  try {
    const json = safeStorage.decryptString(Buffer.from(blob, 'base64'))
    const parsed = JSON.parse(json) as StoredCredentials
    return parsed.email && parsed.password ? parsed : null
  } catch {
    return null
  }
}

export function saveCredentials(slug: string, creds: StoredCredentials): void {
  if (!credentialsAvailable()) throw new Error('OS credential encryption is not available')
  const all = readAll()
  all[slug.toLowerCase()] = safeStorage.encryptString(JSON.stringify(creds)).toString('base64')
  writeAll(all)
}

export function clearCredentials(slug: string): void {
  const all = readAll()
  if (slug.toLowerCase() in all) {
    delete all[slug.toLowerCase()]
    writeAll(all)
  }
}
