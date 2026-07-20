import type {
  ApiEvent, LightgraphData, NightDetail, NightSummary, OverlayGeometry,
  OverlayRequest, Settings, Status,
} from './types'
import { MockApi } from './mock/mockApi'
import { RealApi } from './realApi'

export interface ApiClient {
  login(username: string, password: string): Promise<boolean>
  logout(): Promise<void>
  isAuthenticated(): boolean
  getStatus(): Promise<Status>
  /** raw = full sensor frame without crop (mask still applied) — used by the editor. */
  latestImageUrl(opts?: { raw?: boolean }): string
  subscribe(cb: (e: ApiEvent) => void): () => void
  getLightgraph(): Promise<LightgraphData>
  getOverlay(req: OverlayRequest): Promise<OverlayGeometry>
  getSettings(): Promise<Settings>
  putSettings(s: Settings): Promise<void>
  changePassword(oldPassword: string, newPassword: string): Promise<boolean>
  getNights(): Promise<NightSummary[]>
  getNight(date: string): Promise<NightDetail>
  rebuildNight(date: string): Promise<void>
  deleteNight(date: string): Promise<void>
}

let instance: ApiClient | null = null

/** Swap the client (tests, and Phase 2 will register RealApi here). */
export function setApi(api: ApiClient) {
  instance = api
}

export function getApi(): ApiClient {
  instance ??= import.meta.env.VITE_API_MODE === 'real' ? new RealApi() : new MockApi()
  return instance
}
