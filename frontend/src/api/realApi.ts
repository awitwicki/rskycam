import type { ApiClient } from './client'
import type {
  ApiEvent, DarksLibrary, FocusMeta, FrameMeta, LightgraphData, LogsResponse, NightDetail,
  NightSummary, OverlayGeometry, OverlayRequest, Settings, Status, UpdateInfo,
} from './types'

const AUTH_FLAG = 'rskycam.auth'
const JSON_HEADERS = { 'Content-Type': 'application/json' }

/** An HTTP-level rejection (the server responded, just not with 2xx) —
 * distinct from a network-level failure (fetch rejects, no response at
 * all). Callers that treat "the connection dropped" as "the server is
 * mid-restart" need this distinction (see UpdateWidget's applyUpdate). */
export class HttpError extends Error {
  status: number
  body: string

  constructor(status: number, body: string) {
    super(`HTTP ${status}: ${body}`)
    this.status = status
    this.body = body
  }
}

/** Fetch wrapper: 401 anywhere ⇒ drop the session and notify the app. */
async function http(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(path, init)
  if (res.status === 401) {
    localStorage.removeItem(AUTH_FLAG)
    window.dispatchEvent(new Event('rskycam:unauthorized'))
    throw new Error('unauthorized')
  }
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new HttpError(res.status, body)
  }
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

  async getUpdate(): Promise<UpdateInfo> {
    return http('/api/update').then(json<UpdateInfo>)
  }

  async applyUpdate(): Promise<void> {
    await http('/api/update/apply', { method: 'POST' })
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
    es.addEventListener('focus', (ev) => {
      cb({ type: 'focus', meta: JSON.parse((ev as MessageEvent).data) as FocusMeta })
    })
    return () => es.close()
  }

  getStatus(): Promise<Status> {
    return http('/api/status').then(json<Status>)
  }

  getLightgraph(): Promise<LightgraphData> {
    return http('/api/lightgraph').then(json<LightgraphData>)
  }

  getLogs(lines = 500): Promise<LogsResponse> {
    return http(`/api/logs?lines=${lines}`).then(json<LogsResponse>)
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

  async startDarksCapture(): Promise<void> {
    await http('/api/darks/capture', { method: 'POST' })
  }

  getDarksLibrary(): Promise<DarksLibrary> {
    return http('/api/darks').then(json<DarksLibrary>)
  }

  async clearDarks(): Promise<void> {
    await http('/api/darks', { method: 'DELETE' })
  }

  async setFocus(enabled: boolean, exposureUs?: number, gain?: number): Promise<void> {
    const body: { enabled: boolean; exposureUs?: number; gain?: number } = { enabled }
    if (exposureUs !== undefined) body.exposureUs = exposureUs
    if (gain !== undefined) body.gain = gain
    await http('/api/focus', { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify(body) })
  }

  focusImageUrl(): string {
    return `/api/focus.jpg?ts=${Date.now()}`
  }

  focusStarUrl(): string {
    return `/api/focus/star.png?ts=${Date.now()}`
  }
}
