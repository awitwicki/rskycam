// ── live status ────────────────────────────────────────────────
export interface FrameMeta {
  timestamp: string // ISO 8601
  exposureUs: number
  gain: number
  isNight: boolean
}

export type CaptureState = 'capturing' | 'camera_unavailable' | 'idle' | 'focusing'

// ── focus mode ─────────────────────────────────────────────────
export interface FocusMeta {
  timestamp: string // ISO 8601
  hfd: number | null // null = no star found (clouds, lens cap)
  starX: number // full-frame px of the detected star
  starY: number
  peak: number // 0..255
  saturated: boolean // peak clipped — HFD untrustworthy
  exposureUs: number
  gain: number
}

export interface FocusInfo {
  enabled: boolean
  exposureUs: number
  gain: number
}

export interface CaptureStatus {
  state: CaptureState
  message?: string
  lastFrame?: FrameMeta
}

export interface SensorReading {
  temperatureC: number
  pressureHpa: number
  humidityPct?: number // BMP280 has no humidity
}

export type SensorState =
  | 'disabled' // turned off in settings
  | 'not_detected' // enabled but not answering on the I2C bus
  | 'ok'

export interface SensorStatus {
  state: SensorState
  reading: SensorReading | null // non-null only when state is 'ok'
}

export interface SystemStatus {
  model: string
  cpuTempC: number
  cpuLoadAvg5m: number // 5-minute load average (/proc/loadavg)
  cpuCores: number
  ramUsedMb: number
  ramTotalMb: number
  diskUsedGb: number
  diskTotalGb: number
  uptimeSec: number
  undervoltageNow: boolean
  undervoltageSinceBoot: boolean
}

export interface AstroStatus {
  sunAltDeg: number
  moonAltDeg: number
  moonPhasePct: number // 0..100 illuminated
  moonWaxing: boolean
}

export interface CameraCaps {
  model: string
  maxWidth: number
  maxHeight: number
  minExposureUs: number
}

export interface DarksProgress {
  current: number
  total: number
}

export interface DarkEntry {
  exposureUs: number
  gain: number
  file: string
  capturedAt: string
}

export interface DarksLibrary {
  entries: DarkEntry[]
}

export interface Status {
  version: string
  capture: CaptureStatus
  sensor: SensorStatus
  system: SystemStatus
  astro: AstroStatus
  camera: CameraCaps | null
  darksProgress: DarksProgress | null
  focus: FocusInfo
}

/** GET /api/update — current build vs newest GitHub release. */
export interface UpdateInfo {
  current: string
  latest: string | null
  updateAvailable: boolean
  error: string | null
}

/** Sun altitude sampled across a 24h window (local noon → noon). */
export interface LightgraphData {
  startIso: string
  stepMinutes: number
  sunAltDeg: number[]
}

// ── logs ───────────────────────────────────────────────────────
export interface LogsResponse {
  lines: string[] // tail of the service log, oldest first
}

// ── nights / gallery ───────────────────────────────────────────
export type ArtifactState =
  | { state: 'ready'; url: string; sizeBytes: number }
  | { state: 'generating' }
  | { state: 'error'; message: string }
  | { state: 'skipped'; message: string } // deliberately not produced; message says why
  | { state: 'pending' } // enabled in settings, not generated yet
  | { state: 'disabled' } // turned off in settings

export interface NightSummary {
  date: string // "2026-07-13" — the evening's local date
  frameCount: number
  framesSizeBytes: number // total size of frames/ on disk
  totalSizeBytes: number // frames + keogram/startrails/timelapses, whatever exists
  thumbnailUrl: string
  keogram: ArtifactState
  startrails: ArtifactState
  timelapseDay: ArtifactState
  timelapseNight: ArtifactState
}

export interface FrameInfo {
  timestamp: string
  url: string
  thumbUrl: string // small cached square crop, for the frame grid
  exposureUs: number
  gain: number
}

export interface NightDetail extends NightSummary {
  frames: FrameInfo[]
}

/** POST /api/nights/{date}/detect-pole — detected star-trail circle center. */
export interface PoleDetection {
  poleXPx: number // in the startrails image's own pixel space
  poleYPx: number
  confidence: number // 0..1 vote-peak prominence
}

// ── image geometry ─────────────────────────────────────────────
export type MaskMode = 'circle' | 'none'

/** Sensor-space pixels (uncropped frame). */
export interface CropRect {
  x: number
  y: number
  width: number
  height: number
}

export interface ImageSettings {
  maskMode: MaskMode // 'circle' = black mask outside the manual circle below
  maskCenterXPx: number // mask circle center, sensor-frame px (set by hand)
  maskCenterYPx: number
  maskRadiusPx: number // mask circle radius, px
  crop: CropRect | null // null = full frame; applied last in the pipeline
}

// ── lens calibration ──────────────────────────────────────────
export type LensType = 'fisheye' | 'rectilinear'

export interface LensCalibration {
  lensType: LensType // fisheye: r = f·θ; rectilinear: r = f·tan θ
  focalLengthMm: number
  pixelSizeUm: number // native sensor pixel size (datasheet); binning derived
  pointingAzDeg: number // azimuth of the optical axis, 0–360
  pointingAltDeg: number // altitude of the optical axis; 90 = zenith
  rollDeg: number // rotation about the optical axis (was rotationDeg)
  flip: boolean // mirror east/west
  centerOffsetXPx: number // optical center minus image center
  centerOffsetYPx: number
}

export interface OverlayLayers {
  cardinal: boolean
  altAzGrid: boolean
  raDecGrid: boolean
  constellations: boolean
}

export type TextFieldKind = 'time' | 'exposure' | 'sensorTemp'

export interface OverlayTextField {
  id: string
  kind: TextFieldKind
  x: number
  y: number
  fontSize: number
}

export interface OverlaySettings {
  calibration: LensCalibration
  layers: OverlayLayers
  gridOpacity: number // 0..1, applies to altAz/raDec grid lines
  constellationsOpacity: number // 0..1, applies to constellation lines only
  textFields: OverlayTextField[]
  bakeIntoSavedFrames: boolean
}

export type OverlayLayerId =
  | 'altAz' | 'raDec' | 'cardinal' | 'text' | 'constellations' | 'constellationLabels'

export interface OverlayPolyline {
  layer: OverlayLayerId // 'altAz' | 'raDec' | ...
  points: [number, number][]
  opacity?: number // 0..1; renderer treats missing as 1
}

export interface OverlayLabel {
  layer: OverlayLayerId // 'cardinal' | 'text' | ...
  text: string
  x: number
  y: number
  fontSize: number
  align?: 'center' | 'left'
}

export interface OverlayGeometry {
  imageWidth: number
  imageHeight: number
  polylines: OverlayPolyline[]
  labels: OverlayLabel[]
}

export interface OverlayRequest {
  time?: string // ISO; default now
  calibration?: LensCalibration // override for editor preview
  layers?: OverlayLayers
  gridOpacity?: number // override for editor preview
  constellationsOpacity?: number // override for editor preview
  crop?: CropRect | null // undefined = settings crop; null = sensor space (uncropped)
}

// ── settings ───────────────────────────────────────────────────
export interface CameraSettings {
  driver: 'asi' | 'rpicam' | 'mock'
  autoExposure: boolean
  targetBrightness: number // 0..255 mean target
  exposureUsMin: number
  exposureUsMax: number
  gainMin: number
  gainMax: number
  manualExposureUs: number
  manualGain: number
  intervalSecDay: number
  intervalSecNight: number
  captureDuringDay: boolean
  captureWidth: number // capture resolution (Pi camera); 4:3 keeps the full fisheye view
  captureHeight: number
}

export interface LocationSettings {
  latitudeDeg: number
  longitudeDeg: number
}

export interface SensorSettings {
  enabled: boolean // BME280/BMP280 on I2C
}

export interface ProcessingSettings {
  keogram: boolean
  startrails: boolean
  startrailsBrightnessLimit: number // skip frames brighter than this mean
  timelapseDay: boolean // timelapse of daytime frames
  timelapseNight: boolean // timelapse of nighttime frames
  timelapseFps: number
  timelapseExtraArgs: string // extra ffmpeg args, whitespace-separated
}

export interface StorageSettings {
  framesRetentionDays: number
  artifactsRetentionDays: number
}

export interface DarkFrameSettings {
  enabled: boolean
  minGainToApply: number
  minExposureUsToApply: number
}

export interface Settings {
  camera: CameraSettings
  image: ImageSettings
  location: LocationSettings
  sensor: SensorSettings
  overlay: OverlaySettings
  processing: ProcessingSettings
  storage: StorageSettings
  darks: DarkFrameSettings
}

// ── events ─────────────────────────────────────────────────────
export type ApiEvent =
  | { type: 'frame'; imageUrl: string; meta: FrameMeta }
  | { type: 'status'; status: Status }
  | { type: 'focus'; meta: FocusMeta }
