import { describe, it, expect } from 'vitest'
import type {
  ApiEvent, ArtifactState, CaptureState, FocusMeta, OverlayGeometry, Settings, Status, UpdateInfo,
} from './types'

describe('api contract', () => {
  it('accepts a fully-populated Status', () => {
    const s: Status = {
      version: '0.5.0.7',
      capture: {
        state: 'capturing',
        lastFrame: { timestamp: '2026-07-14T01:00:00Z', exposureUs: 30_000_000, gain: 250, isNight: true },
      },
      astro: { sunAltDeg: -32.5, moonAltDeg: 12.1, moonPhasePct: 64, moonWaxing: true },
      camera: { model: 'ZWO ASI120MM Mini', maxWidth: 1280, maxHeight: 960, minExposureUs: 32 },
      sensor: { state: 'ok', reading: { temperatureC: 8.4, pressureHpa: 1013.2, humidityPct: 62 } },
      system: {
        model: 'Raspberry Pi 4 Model B Rev 1.4', cpuTempC: 52, cpuLoadAvg5m: 2.25, cpuCores: 4,
        ramUsedMb: 1200, ramTotalMb: 3906, diskUsedGb: 41, diskTotalGb: 118, uptimeSec: 260000,
        undervoltageNow: false, undervoltageSinceBoot: true,
      },
      darksProgress: { current: 3, total: 15 },
      focus: { enabled: true, exposureUs: 1_000_000, gain: 8 },
    }
    expect(s.sensor.reading?.humidityPct).toBe(62)

    const offStates: Status['sensor'][] = [
      { state: 'disabled', reading: null },
      { state: 'not_detected', reading: null },
    ]
    expect(offStates.every((x) => x.reading === null)).toBe(true)
  })

  it('accepts UpdateInfo in both states', () => {
    const yes: UpdateInfo = { current: '0.5.0.7', latest: 'v0.5.0.9', updateAvailable: true, error: null }
    const err: UpdateInfo = { current: '0.5.0-dev', latest: null, updateAvailable: false, error: 'offline' }
    expect(yes.updateAvailable).toBe(true)
    expect(err.latest).toBeNull()
  })

  it('accepts every ArtifactState variant', () => {
    const all: ArtifactState[] = [
      { state: 'ready', url: '/x.jpg', sizeBytes: 12_345 },
      { state: 'generating' },
      { state: 'error', message: 'boom' },
      { state: 'pending' },
      { state: 'disabled' },
    ]
    expect(all).toHaveLength(5)
  })

  it('accepts an OverlayGeometry and a Settings literal', () => {
    const g: OverlayGeometry = {
      imageWidth: 960, imageHeight: 960,
      polylines: [{ layer: 'altAz', points: [[0, 0], [1, 1]] }],
      labels: [{ layer: 'cardinal', text: 'N', x: 480, y: 30, fontSize: 28 }],
    }
    const st: Settings = {
      camera: {
        driver: 'mock', autoExposure: true, targetBrightness: 100,
        exposureUsMin: 32, exposureUsMax: 60_000_000, gainMin: 0, gainMax: 300,
        manualExposureUs: 30_000_000, manualGain: 250, intervalSecDay: 120, intervalSecNight: 60,
        captureDuringDay: false,
        captureWidth: 1640, captureHeight: 1232,
      },
      image: {
        maskMode: 'circle', maskCenterXPx: 640, maskCenterYPx: 480, maskRadiusPx: 620,
        crop: { x: 160, y: 120, width: 960, height: 720 },
      },
      location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
      sensor: { enabled: true },
      overlay: {
        calibration: {
          lensType: 'fisheye', focalLengthMm: 1.8, pixelSizeUm: 1.12,
          pointingAzDeg: 0, pointingAltDeg: 90, rollDeg: 0, flip: false,
          centerOffsetXPx: 0, centerOffsetYPx: 0,
        },
        layers: { cardinal: true, altAzGrid: true, raDecGrid: true, constellations: false },
        gridOpacity: 0.45,
        constellationsOpacity: 0.55,
        textFields: [{ id: 'time', kind: 'time', x: 24, y: 40, fontSize: 24 }],
        bakeIntoSavedFrames: false,
      },
      processing: { keogram: true, startrails: true, startrailsBrightnessLimit: 35, timelapseDay: true, timelapseNight: true, timelapseFps: 25, timelapseExtraArgs: '' },
      storage: { framesRetentionDays: 14, artifactsRetentionDays: 60 },
      darks: { enabled: false, minGainToApply: 15, minExposureUsToApply: 10_000_000 },
    }
    expect(g.polylines[0].layer).toBe('altAz')
    expect(st.camera.driver).toBe('mock')
    expect(st.image.maskMode).toBe('circle')

    const noMaskNoCrop: Settings['image'] = {
      maskMode: 'none', maskCenterXPx: 640, maskCenterYPx: 480, maskRadiusPx: 620, crop: null,
    }
    expect(noMaskNoCrop.crop).toBeNull()
  })

  it('accepts focus meta and the focusing capture state', () => {
    const m: FocusMeta = {
      timestamp: '2026-08-01T22:00:00Z', hfd: 4.2, starX: 512, starY: 384,
      peak: 213, saturated: false, exposureUs: 1_000_000, gain: 8,
    }
    const noStar: FocusMeta = { ...m, hfd: null }
    expect(noStar.hfd).toBeNull()
    const st: CaptureState = 'focusing'
    expect(st).toBe('focusing')
    const ev: ApiEvent = { type: 'focus', meta: m }
    expect(ev.type).toBe('focus')
  })
})
