// ── live status ────────────────────────────────────────────────
export interface FrameMeta {
  timestamp: string // ISO 8601
  exposureUs: number
  gain: number
  isNight: boolean
}

export type CaptureState = 'capturing' | 'camera_unavailable' | 'idle'

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
}

export interface Status {
  capture: CaptureStatus
  sensor: SensorStatus
  system: SystemStatus
  astro: AstroStatus
  camera: CameraCaps | null
}

/** Sun altitude sampled across a 24h window (local noon → noon). */
export interface LightgraphData {
  startIso: string
  stepMinutes: number
  sunAltDeg: number[]
}

// ── nights / gallery ───────────────────────────────────────────
export type ArtifactState =
  | { state: 'ready'; url: string }
  | { state: 'generating' }
  | { state: 'error'; message: string }
  | { state: 'pending' } // enabled in settings, not generated yet
  | { state: 'disabled' } // turned off in settings

export interface NightSummary {
  date: string // "2026-07-13" — the evening's local date
  frameCount: number
  thumbnailUrl: string
  keogram: ArtifactState
  startrails: ArtifactState
  timelapse: ArtifactState
}

export interface FrameInfo {
  timestamp: string
  url: string
  exposureUs: number
  gain: number
}

export interface NightDetail extends NightSummary {
  frames: FrameInfo[]
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
  maskMode: MaskMode // 'circle' = black mask outside the lens circle
  crop: CropRect | null // null = full frame; applied last in the pipeline
}

// ── overlay ────────────────────────────────────────────────────
export interface LensCalibration {
  cx: number // px, source-image coords
  cy: number
  radiusPx: number // horizon circle radius
  rotationDeg: number // where north points in the image
  flip: boolean // mirror east/west
}

export interface OverlayLayers {
  cardinal: boolean
  altAzGrid: boolean
  raDecGrid: boolean
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
  textFields: OverlayTextField[]
  bakeIntoSavedFrames: boolean
}

export type OverlayLayerId = 'altAz' | 'raDec' | 'cardinal' | 'text'

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
  intervalSec: number
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
  timelapse: boolean
  timelapseFps: number
  timelapseExtraArgs: string // extra ffmpeg args, whitespace-separated
}

export interface StorageSettings {
  framesRetentionDays: number
  artifactsRetentionDays: number
}

export interface Settings {
  camera: CameraSettings
  image: ImageSettings
  location: LocationSettings
  sensor: SensorSettings
  overlay: OverlaySettings
  processing: ProcessingSettings
  storage: StorageSettings
}

// ── events ─────────────────────────────────────────────────────
export type ApiEvent =
  | { type: 'frame'; imageUrl: string; meta: FrameMeta }
  | { type: 'status'; status: Status }
