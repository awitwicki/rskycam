import { describe, it, expect } from 'vitest'
import type { ArtifactState, OverlayGeometry, Settings, Status } from './types'

describe('api contract', () => {
  it('accepts a fully-populated Status', () => {
    const s: Status = {
      capture: {
        state: 'capturing',
        lastFrame: { timestamp: '2026-07-14T01:00:00Z', exposureUs: 30_000_000, gain: 250, isNight: true },
      },
      astro: { sunAltDeg: -32.5, moonAltDeg: 12.1, moonPhasePct: 64, moonWaxing: true },
      camera: { model: 'ZWO ASI120MM Mini', maxWidth: 1280, maxHeight: 960 },
      sensor: { state: 'ok', reading: { temperatureC: 8.4, pressureHpa: 1013.2, humidityPct: 62 } },
      system: {
        model: 'Raspberry Pi 4 Model B Rev 1.4', cpuTempC: 52, ramUsedMb: 1200,
        ramTotalMb: 3906, diskUsedGb: 41, diskTotalGb: 118, uptimeSec: 260000,
        undervoltageNow: false, undervoltageSinceBoot: true,
      },
    }
    expect(s.sensor.reading?.humidityPct).toBe(62)

    const offStates: Status['sensor'][] = [
      { state: 'disabled', reading: null },
      { state: 'not_detected', reading: null },
    ]
    expect(offStates.every((x) => x.reading === null)).toBe(true)
  })

  it('accepts every ArtifactState variant', () => {
    const all: ArtifactState[] = [
      { state: 'ready', url: '/x.jpg' },
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
        manualExposureUs: 30_000_000, manualGain: 250, intervalSec: 60, captureDuringDay: false,
        captureWidth: 1640, captureHeight: 1232,
      },
      image: { maskMode: 'circle', crop: { x: 160, y: 120, width: 960, height: 720 } },
      location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
      sensor: { enabled: true },
      overlay: {
        calibration: { cx: 480, cy: 480, radiusPx: 440, rotationDeg: 0, flip: false },
        layers: { cardinal: true, altAzGrid: true, raDecGrid: true },
        gridOpacity: 0.45,
        textFields: [{ id: 'time', kind: 'time', x: 24, y: 40, fontSize: 24 }],
        bakeIntoSavedFrames: false,
      },
      processing: { keogram: true, startrails: true, startrailsBrightnessLimit: 35, timelapse: true, timelapseFps: 25, timelapseExtraArgs: '' },
      storage: { framesRetentionDays: 14, artifactsRetentionDays: 60 },
    }
    expect(g.polylines[0].layer).toBe('altAz')
    expect(st.camera.driver).toBe('mock')
    expect(st.image.maskMode).toBe('circle')

    const noMaskNoCrop: Settings['image'] = { maskMode: 'none', crop: null }
    expect(noMaskNoCrop.crop).toBeNull()
  })
})
