import type { ApiClient } from './client'
import type {
  ApiEvent, FrameMeta, LightgraphData, NightDetail, NightSummary, OverlayGeometry,
  OverlayRequest, Settings, Status,
} from './types'

const AUTH_FLAG = 'rskycam.auth'
const JSON_HEADERS = { 'Content-Type': 'application/json' }

/** Fetch wrapper: 401 anywhere ⇒ drop the session and notify the app. */
async function http(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(path, init)
  if (res.status === 401) {
    localStorage.removeItem(AUTH_FLAG)
    window.dispatchEvent(new Event('rskycam:unauthorized'))
    throw new Error('unauthorized')
  }
  if (!res.ok) throw new Error(`${init?.method ?? 'GET'} ${path} failed: ${res.status}`)
  return res
}

const json = <T>(r: Response) => r.json() as Promise<T>

export class RealApi implements ApiClient {
  async login(username: string, password: string): Promise<boolean> {
    const res = await fetch('/api/login', {
      method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ username, password }),
    })
    if (!res.ok) return false
    localStorage.setItem(AUTH_FLAG, '1')
    return true
  }

  async logout(): Promise<void> {
    localStorage.removeItem(AUTH_FLAG)
    await fetch('/api/logout', { method: 'POST' })
  }

  isAuthenticated(): boolean {
    return localStorage.getItem(AUTH_FLAG) === '1'
  }

  async changePassword(oldPassword: string, newPassword: string): Promise<boolean> {
    const res = await fetch('/api/change-password', {
      method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ oldPassword, newPassword }),
    })
    return res.ok
  }

  latestImageUrl(opts?: { raw?: boolean }): string {
    return `/api/latest.jpg?raw=${opts?.raw ? 1 : 0}&ts=${Date.now()}`
  }

  subscribe(cb: (e: ApiEvent) => void): () => void {
    const es = new EventSource('/api/events')
    es.addEventListener('frame', (ev) => {
      const d = JSON.parse((ev as MessageEvent).data) as { imageUrl: string; meta: FrameMeta }
      cb({ type: 'frame', imageUrl: d.imageUrl, meta: d.meta })
    })
    es.addEventListener('status', (ev) => {
      cb({ type: 'status', status: JSON.parse((ev as MessageEvent).data) as Status })
    })
    return () => es.close()
  }

  getStatus(): Promise<Status> {
    return http('/api/status').then(json<Status>)
  }

  getLightgraph(): Promise<LightgraphData> {
    return http('/api/lightgraph').then(json<LightgraphData>)
  }

  getOverlay(req: OverlayRequest): Promise<OverlayGeometry> {
    // JSON.stringify drops undefined keys, keeps null — exactly the crop tri-state.
    return http('/api/overlay', {
      method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(req),
    }).then(json<OverlayGeometry>)
  }

  getSettings(): Promise<Settings> {
    return http('/api/settings').then(json<Settings>)
  }

  async putSettings(s: Settings): Promise<void> {
    await http('/api/settings', { method: 'PUT', headers: JSON_HEADERS, body: JSON.stringify(s) })
  }

  getNights(): Promise<NightSummary[]> {
    return http('/api/nights').then(json<NightSummary[]>)
  }

  getNight(date: string): Promise<NightDetail> {
    return http(`/api/nights/${date}`).then(json<NightDetail>)
  }

  async rebuildNight(date: string): Promise<void> {
    await http(`/api/nights/${date}/rebuild`, { method: 'POST' })
  }

  async deleteNight(date: string): Promise<void> {
    await http(`/api/nights/${date}`, { method: 'DELETE' })
  }
}
