import type { ApiClient } from '../client'
import type {
  ApiEvent, ArtifactState, FrameInfo, FrameMeta, LightgraphData, NightDetail,
  NightSummary, OverlayGeometry, OverlayRequest, Settings, Status, TextFieldKind,
} from '../types'
import {
  altitudeOf, moonEquatorial, moonIllumination, sunEquatorial,
} from '../../lib/astro'
import { formatExposure, formatGain } from '../../lib/format'
import { buildOverlayGeometry, cropGeometry } from '../../lib/overlayGeometry'
import { renderKeogram, renderStartrails } from './artifacts'
import { renderSky, SENSOR_H, SENSOR_W } from './skyImage'

const SESSION_KEY = 'rskycam.mock.session'
const PASSWORD_KEY = 'rskycam.mock.password'
const SETTINGS_KEY = 'rskycam.mock.settings'
const DEFAULT_PASSWORD = 'pa$$word!0'
const SAMPLE_FRAME_URL = '/mock/real-frame.jpg'

export function defaultSettings(): Settings {
  return {
    camera: {
      driver: 'mock', autoExposure: true, targetBrightness: 100,
      exposureUsMin: 32, exposureUsMax: 60_000_000, gainMin: 0, gainMax: 300,
      manualExposureUs: 30_000_000, manualGain: 250, intervalSec: 60, captureDuringDay: false,
      captureWidth: 1640, captureHeight: 1232,
    },
    image: { maskMode: 'none', crop: null },
    location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
    sensor: { enabled: true },
    overlay: {
      // Fisheye image circle larger than the sensor, like the real ASI120MM frame.
      calibration: { cx: 640, cy: 480, radiusPx: 620, rotationDeg: 0, flip: false },
      layers: { cardinal: true, altAzGrid: true, raDecGrid: true },
      gridOpacity: 0.45,
      textFields: [
        { id: 'time', kind: 'time', x: 24, y: 40, fontSize: 24 },
        { id: 'exposure', kind: 'exposure', x: 24, y: 72, fontSize: 18 },
      ],
      bakeIntoSavedFrames: false,
    },
    processing: { keogram: true, startrails: true, startrailsBrightnessLimit: 35, timelapse: true, timelapseFps: 25, timelapseExtraArgs: '' },
    storage: { framesRetentionDays: 14, artifactsRetentionDays: 60 },
  }
}

function localDateStr(d: Date): string {
  const m = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  return `${d.getFullYear()}-${m}-${day}`
}

export class MockApi implements ApiClient {
  private readonly renderFrame: (time: Date, size: number) => string
  private readonly renderRaw: (time: Date, size: number) => string
  private readonly sample: HTMLImageElement | null = null
  private readonly startedAt = Date.now()
  private cpuTemp = 52
  private lastFrameTime: Date | null = null
  private lastFrameUrl: string | null = null
  private lastRawUrl: string | null = null
  private nightList: NightSummary[] | null = null
  private readonly nightFrames = new Map<string, FrameInfo[]>()

  constructor(opts: { renderFrame?: (time: Date, size: number) => string } = {}) {
    if (!opts.renderFrame && typeof Image !== 'undefined') {
      this.sample = new Image()
      this.sample.src = SAMPLE_FRAME_URL // falls back to synthetic sky until loaded
    }
    const render = (crop: 'settings' | 'none') => (time: Date, size: number) => {
      const s = this.settings()
      return renderSky({
        time,
        location: s.location,
        calibration: s.overlay.calibration,
        maskMode: s.image.maskMode,
        crop: crop === 'settings' ? s.image.crop : null,
        scale: size / SENSOR_W,
        photo: this.sample,
      })
    }
    this.renderFrame = opts.renderFrame ?? render('settings')
    this.renderRaw = opts.renderFrame ?? render('none')
  }

  private settings(): Settings {
    const raw = localStorage.getItem(SETTINGS_KEY)
    if (!raw) return defaultSettings()
    // Merge so settings stored by older versions gain newly added sections
    // and newly added fields inside the overlay section.
    const stored = JSON.parse(raw) as Partial<Settings>
    const d = defaultSettings()
    return { ...d, ...stored, overlay: { ...d.overlay, ...stored.overlay } }
  }

  // ── auth ──
  async login(username: string, password: string): Promise<boolean> {
    const pw = localStorage.getItem(PASSWORD_KEY) ?? DEFAULT_PASSWORD
    if (username !== 'admin' || password !== pw) return false
    sessionStorage.setItem(SESSION_KEY, 'admin')
    return true
  }

  async logout(): Promise<void> {
    sessionStorage.removeItem(SESSION_KEY)
  }

  isAuthenticated(): boolean {
    return sessionStorage.getItem(SESSION_KEY) === 'admin'
  }

  async changePassword(oldPassword: string, newPassword: string): Promise<boolean> {
    const pw = localStorage.getItem(PASSWORD_KEY) ?? DEFAULT_PASSWORD
    if (oldPassword !== pw) return false
    localStorage.setItem(PASSWORD_KEY, newPassword)
    return true
  }

  // ── live status ──
  private frameMeta(time: Date): FrameMeta {
    return { timestamp: time.toISOString(), exposureUs: 30_000_000, gain: 250, isNight: true }
  }

  private astroNow(): Status['astro'] {
    const { latitudeDeg, longitudeDeg } = this.settings().location
    const now = new Date()
    const sun = sunEquatorial(now)
    const moon = moonEquatorial(now)
    const ill = moonIllumination(now)
    return {
      sunAltDeg: altitudeOf(now, sun.raDeg, sun.decDeg, latitudeDeg, longitudeDeg),
      moonAltDeg: altitudeOf(now, moon.raDeg, moon.decDeg, latitudeDeg, longitudeDeg),
      moonPhasePct: ill.pct,
      moonWaxing: ill.waxing,
    }
  }

  async getLightgraph(): Promise<LightgraphData> {
    const { latitudeDeg, longitudeDeg } = this.settings().location
    const start = new Date()
    start.setHours(12, 0, 0, 0)
    if (Date.now() < start.getTime()) start.setDate(start.getDate() - 1)
    const stepMinutes = 10
    const sunAltDeg: number[] = []
    for (let i = 0; i < (24 * 60) / stepMinutes; i++) {
      const t = new Date(start.getTime() + i * stepMinutes * 60_000)
      const sun = sunEquatorial(t)
      sunAltDeg.push(altitudeOf(t, sun.raDeg, sun.decDeg, latitudeDeg, longitudeDeg))
    }
    return { startIso: start.toISOString(), stepMinutes, sunAltDeg }
  }

  async getStatus(): Promise<Status> {
    this.cpuTemp = Math.min(72, Math.max(42, this.cpuTemp + (Math.random() - 0.5) * 1.5))
    return {
      astro: this.astroNow(),
      capture: { state: 'capturing', lastFrame: this.frameMeta(new Date()) },
      camera: { model: 'Mock synthetic sky', maxWidth: 1280, maxHeight: 960 },
      sensor: this.settings().sensor.enabled
        ? {
            state: 'ok',
            reading: {
              temperatureC: 8.4 + Math.random(),
              pressureHpa: 1013 + Math.random() * 2,
              humidityPct: 62 + Math.random() * 3,
            },
          }
        : { state: 'disabled', reading: null },
      system: {
        model: 'Raspberry Pi 4 Model B Rev 1.4',
        cpuTempC: this.cpuTemp,
        ramUsedMb: 1210 + Math.round(Math.random() * 80),
        ramTotalMb: 3906,
        diskUsedGb: 41.2,
        diskTotalGb: 118.0,
        uptimeSec: Math.floor((Date.now() - this.startedAt) / 1000) + 86_400 * 3,
        undervoltageNow: false,
        undervoltageSinceBoot: true,
      },
    }
  }

  latestImageUrl(opts?: { raw?: boolean }): string {
    if (!this.lastFrameUrl) this.renderAndCache(new Date())
    if (opts?.raw) {
      this.lastRawUrl ??= this.renderRaw(this.lastFrameTime ?? new Date(), SENSOR_W)
      return this.lastRawUrl
    }
    return this.lastFrameUrl!
  }

  private renderAndCache(t: Date): string {
    this.lastFrameTime = t
    this.lastRawUrl = null // re-rendered on demand for the editor
    this.lastFrameUrl = this.renderFrame(t, SENSOR_W)
    return this.lastFrameUrl
  }

  subscribe(cb: (e: ApiEvent) => void): () => void {
    const emitFrame = () => {
      const t = new Date()
      cb({ type: 'frame', imageUrl: this.renderAndCache(t), meta: this.frameMeta(t) })
    }
    const emitStatus = () => void this.getStatus().then((status) => cb({ type: 'status', status }))
    queueMicrotask(() => {
      emitFrame()
      emitStatus()
    })
    const f = setInterval(emitFrame, 5000)
    const s = setInterval(emitStatus, 2500)
    return () => {
      clearInterval(f)
      clearInterval(s)
    }
  }

  // ── overlay ──
  private textFor(kind: TextFieldKind): string {
    if (kind === 'time') return new Date().toLocaleString()
    if (kind === 'exposure') return `exp ${formatExposure(30_000_000)} · gain ${formatGain(250)}`
    return this.settings().sensor.enabled ? '8.4°C' : '—°C'
  }

  async getOverlay(req: OverlayRequest): Promise<OverlayGeometry> {
    const s = this.settings()
    const geo = buildOverlayGeometry({
      time: req.time ? new Date(req.time) : new Date(),
      location: s.location,
      calibration: req.calibration ?? s.overlay.calibration,
      layers: req.layers ?? s.overlay.layers,
      gridOpacity: req.gridOpacity ?? s.overlay.gridOpacity,
      imageWidth: SENSOR_W,
      imageHeight: SENSOR_H,
    })
    for (const f of s.overlay.textFields) {
      geo.labels.push({
        layer: 'text', text: this.textFor(f.kind),
        x: f.x, y: f.y, fontSize: f.fontSize, align: 'left',
      })
    }
    const crop = req.crop !== undefined ? req.crop : s.image.crop
    return crop ? cropGeometry(geo, crop) : geo
  }

  // ── settings ──
  async getSettings(): Promise<Settings> {
    return this.settings()
  }

  async putSettings(next: Settings): Promise<void> {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(next))
  }

  // ── nights ──
  private buildList(): NightSummary[] {
    if (this.nightList) return this.nightList
    const out: NightSummary[] = []
    for (let i = 1; i <= 8; i++) {
      const d = new Date()
      d.setDate(d.getDate() - i)
      d.setHours(22, 0, 0, 0)
      const keogram: ArtifactState =
        i === 4 ? { state: 'disabled' } : { state: 'ready', url: renderKeogram(720, 220, i) }
      const timelapse: ArtifactState =
        i === 1 ? { state: 'generating' }
        : i === 2 ? { state: 'error', message: 'ffmpeg exited with code 1' }
        : { state: 'ready', url: '/mock/timelapse.mp4' }
      out.push({
        date: localDateStr(d),
        frameCount: 640 - i * 7,
        thumbnailUrl: this.renderFrame(d, 240),
        keogram,
        startrails: { state: 'ready', url: renderStartrails(720, 100 + i) },
        timelapse,
      })
    }
    this.nightList = out
    return out
  }

  async getNights(): Promise<NightSummary[]> {
    return this.buildList()
  }

  async getNight(date: string): Promise<NightDetail> {
    const n = this.buildList().find((x) => x.date === date)
    if (!n) throw new Error(`night not found: ${date}`)
    let frames = this.nightFrames.get(date)
    if (!frames) {
      frames = Array.from({ length: 12 }, (_, k) => {
        const t = new Date(`${date}T22:00:00`)
        t.setMinutes(k * 30)
        return {
          timestamp: t.toISOString(),
          url: this.renderFrame(t, 480),
          exposureUs: 30_000_000,
          gain: 250,
        }
      })
      this.nightFrames.set(date, frames)
    }
    return { ...n, frames }
  }

  async rebuildNight(date: string): Promise<void> {
    const n = this.buildList().find((x) => x.date === date)
    if (!n) throw new Error(`night not found: ${date}`)
    n.timelapse = { state: 'generating' }
  }

  async deleteNight(date: string): Promise<void> {
    const list = this.buildList()
    if (!list.some((x) => x.date === date)) throw new Error(`night not found: ${date}`)
    this.nightList = list.filter((x) => x.date !== date)
  }
}
