import type {
  ApiEvent, DarksLibrary, LightgraphData, LogsResponse, NightDetail, NightSummary,
  OverlayGeometry, OverlayRequest, Settings, Status, UpdateInfo,
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
  getLogs(lines?: number): Promise<LogsResponse>
  getOverlay(req: OverlayRequest): Promise<OverlayGeometry>
  getSettings(): Promise<Settings>
  putSettings(s: Settings): Promise<void>
  changePassword(oldPassword: string, newPassword: string): Promise<boolean>
  getUpdate(): Promise<UpdateInfo>
  applyUpdate(): Promise<void>
  getNights(): Promise<NightSummary[]>
  getNight(date: string): Promise<NightDetail>
  rebuildNight(date: string): Promise<void>
  deleteNight(date: string): Promise<void>
  startDarksCapture(): Promise<void>
  getDarksLibrary(): Promise<DarksLibrary>
  clearDarks(): Promise<void>
  setFocus(enabled: boolean, exposureUs?: number, gain?: number): Promise<void>
  focusImageUrl(): string
  focusStarUrl(): string
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
