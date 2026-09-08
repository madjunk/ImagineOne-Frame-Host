import { session } from 'electron'
import type { CurrentUser, TenantBranding } from '../shared/types'

/** The app view and all API calls share this persistent cookie store. */
export const APP_PARTITION = 'persist:imagineone'

export function appSession() {
  return session.fromPartition(APP_PARTITION)
}

/**
 * fetch() bound to the app partition, so the server's httpOnly `auth_token`
 * cookie is sent automatically — the frame never reads or stores the token.
 */
async function apiFetch(baseUrl: string, path: string, init?: RequestInit): Promise<Response> {
  return appSession().fetch(`${baseUrl}${path}`, { ...init, credentials: 'include' })
}

export async function checkDeployment(baseUrl: string): Promise<{ instanceRole: string; pwaRequired: boolean }> {
  const res = await apiFetch(baseUrl, '/api/public/deployment')
  if (!res.ok) throw new Error(`Deployment check failed (${res.status})`)
  return res.json()
}

export async function fetchTenant(baseUrl: string, slug: string): Promise<TenantBranding | null> {
  const res = await apiFetch(baseUrl, `/api/public/tenant/${encodeURIComponent(slug)}`)
  if (res.status === 404) return null
  if (!res.ok) throw new Error(`Tenant lookup failed (${res.status})`)
  return res.json()
}

export async function fetchMe(baseUrl: string): Promise<CurrentUser | null> {
  try {
    const res = await apiFetch(baseUrl, '/api/auth/me')
    if (res.status === 401) return null
    if (!res.ok) return null
    return res.json()
  } catch {
    return null
  }
}

/** Tenant-scoped settings for the signed-in user (theme-*, company-* keys). */
export async function fetchSettings(baseUrl: string): Promise<Record<string, string | null> | null> {
  const res = await apiFetch(baseUrl, '/api/settings')
  if (!res.ok) return null
  return res.json()
}

/** Public branding, optionally for one slug; used before sign-in. */
export async function fetchSiteSettings(baseUrl: string, slug?: string): Promise<Record<string, unknown> | null> {
  const q = slug ? `?slug=${encodeURIComponent(slug)}` : ''
  const res = await apiFetch(baseUrl, `/api/public/site-settings${q}`)
  if (!res.ok) return null
  return res.json()
}

export async function logout(baseUrl: string): Promise<void> {
  try {
    await apiFetch(baseUrl, '/api/auth/logout', { method: 'POST' })
  } catch {
    // Server unreachable — still clear local cookies below.
  }
  await appSession().clearStorageData({ storages: ['cookies'] })
}
