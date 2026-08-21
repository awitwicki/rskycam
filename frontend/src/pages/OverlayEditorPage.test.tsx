import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { setApi } from '../api/client'
import type { ApiClient } from '../api/client'
import type { NightSummary, Settings, Status } from '../api/types'
import OverlayEditorPage from './OverlayEditorPage'

const settings: Settings = {
  camera: {
    driver: 'mock', autoExposure: true, targetBrightness: 100,
    exposureUsMin: 100, exposureUsMax: 30_000_000, gainMin: 0, gainMax: 16,
    manualExposureUs: 1_000_000, manualGain: 1, intervalSecDay: 60,
    intervalSecNight: 60, captureDuringDay: true, captureWidth: 1280, captureHeight: 960,
  },
  image: { maskMode: 'none', maskCenterXPx: 640, maskCenterYPx: 480, maskRadiusPx: 620, crop: null },
  location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
  sensor: { enabled: false },
  overlay: {
    calibration: {
      lensType: 'fisheye', focalLengthMm: 1.48, pixelSizeUm: 3.75,
      pointingAzDeg: 0, pointingAltDeg: 90, rollDeg: 0, flip: false,
      centerOffsetXPx: 0, centerOffsetYPx: 0,
    },
    layers: { cardinal: true, altAzGrid: true, raDecGrid: true, constellations: false },
    gridOpacity: 0.45,
    constellationsOpacity: 0.55,
    textFields: [],
    bakeIntoSavedFrames: false,
  },
  processing: {
    keogram: true, startrails: true, startrailsBrightnessLimit: 35,
    timelapseDay: true, timelapseNight: true, timelapseFps: 25, timelapseExtraArgs: '',
  },
  storage: { framesRetentionDays: 14, artifactsRetentionDays: 60 },
  darks: { enabled: false, minGainToApply: 15, minExposureUsToApply: 10_000_000 },
}

const status: Status = {
  version: 'test',
  capture: { state: 'capturing' },
  sensor: { state: 'disabled', reading: null },
  system: {
    model: 'test', cpuTempC: 50, cpuLoadAvg5m: 1, cpuCores: 4,
    ramUsedMb: 500, ramTotalMb: 2000, diskUsedGb: 10, diskTotalGb: 100,
    uptimeSec: 60, undervoltageNow: false, undervoltageSinceBoot: false,
  },
  astro: { sunAltDeg: -20, moonAltDeg: 10, moonPhasePct: 50, moonWaxing: true },
  camera: null,
  darksProgress: null,
  focus: { enabled: false, exposureUs: 1_000_000, gain: 1 },
}

const night = (date: string, startrails: NightSummary['startrails']): NightSummary => ({
  date,
  frameCount: 100,
  framesSizeBytes: 1_000_000,
  totalSizeBytes: 1_000_000,
  thumbnailUrl: `/api/files/${date}/frames/x.jpg?thumb=1`,
  keogram: { state: 'pending' },
  startrails,
  timelapseDay: { state: 'pending' },
  timelapseNight: { state: 'pending' },
})

function setup(nights: NightSummary[], extra: Partial<ApiClient> = {}) {
  const getNights = vi.fn<() => Promise<NightSummary[]>>().mockResolvedValue(nights)
  setApi({
    getSettings: () => Promise.resolve(settings),
    getStatus: () => Promise.resolve(status),
    subscribe: () => () => {},
    latestImageUrl: () => '/api/latest.jpg?raw=1',
    getNights,
    ...extra,
  } as unknown as ApiClient)
  render(<OverlayEditorPage />)
  return { getNights }
}

afterEach(cleanup)

describe('OverlayEditorPage startrails background', () => {
  it('lists only nights with a ready startrails in the picker', async () => {
    const { getNights } = setup([
      night('2026-08-16', { state: 'ready', url: '/api/files/2026-08-16/startrails.jpg', sizeBytes: 1_000_000 }),
      night('2026-08-15', { state: 'pending' }),
    ])

    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    expect(getNights).toHaveBeenCalledTimes(1)
    expect(await screen.findByText('2026-08-16')).toBeInTheDocument()
    expect(screen.queryByText('2026-08-15')).not.toBeInTheDocument()
    // Thumbnails go through the cached-thumb endpoint, not the full JPEG.
    const thumb = screen.getByAltText(/startrails thumbnail 2026-08-16/i)
    expect(thumb.getAttribute('src')).toBe('/api/files/2026-08-16/startrails.jpg?thumb=1')
  })

  it('shows an empty hint when no night has a startrails yet', async () => {
    setup([night('2026-08-15', { state: 'pending' })])
    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    expect(await screen.findByText(/no startrails yet/i)).toBeInTheDocument()
  })

  it('selects a night as background and returns to live', async () => {
    setup([
      night('2026-08-16', { state: 'ready', url: '/api/files/2026-08-16/startrails.jpg', sizeBytes: 1_000_000 }),
    ])

    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    await userEvent.click(await screen.findByText('2026-08-16'))

    const bg = screen.getByAltText('Startrails 2026-08-16')
    expect(bg.getAttribute('src')).toBe('/api/files/2026-08-16/startrails.jpg')

    await userEvent.click(screen.getByRole('button', { name: /^live$/i }))
    expect(screen.queryByAltText('Startrails 2026-08-16')).not.toBeInTheDocument()
  })

  it('warns when the startrails size matches neither sensor nor crop', async () => {
    setup([
      night('2026-08-16', { state: 'ready', url: '/api/files/2026-08-16/startrails.jpg', sizeBytes: 1_000_000 }),
    ])

    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    await userEvent.click(await screen.findByText('2026-08-16'))

    const bg = screen.getByAltText('Startrails 2026-08-16')
    Object.defineProperty(bg, 'naturalWidth', { value: 720 })
    Object.defineProperty(bg, 'naturalHeight', { value: 720 })
    fireEvent.load(bg)
    expect(await screen.findByText(/alignment may be off/i)).toBeInTheDocument()
  })

  it('shows no warning when the startrails matches the sensor size', async () => {
    setup([
      night('2026-08-16', { state: 'ready', url: '/api/files/2026-08-16/startrails.jpg', sizeBytes: 1_000_000 }),
    ])

    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    await userEvent.click(await screen.findByText('2026-08-16'))

    const bg = screen.getByAltText('Startrails 2026-08-16')
    // Default editor frame dims (no live frame loaded in jsdom) are 1280×960.
    Object.defineProperty(bg, 'naturalWidth', { value: 1280 })
    Object.defineProperty(bg, 'naturalHeight', { value: 960 })
    fireEvent.load(bg)
    expect(screen.queryByText(/alignment may be off/i)).not.toBeInTheDocument()
  })
})

describe('OverlayEditorPage auto-align', () => {
  const ready = night('2026-08-16',
    { state: 'ready', url: '/api/files/2026-08-16/startrails.jpg', sizeBytes: 1_000_000 })

  async function selectNight() {
    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    await userEvent.click(await screen.findByText('2026-08-16'))
    const bg = screen.getByAltText('Startrails 2026-08-16')
    Object.defineProperty(bg, 'naturalWidth', { value: 1280 })
    Object.defineProperty(bg, 'naturalHeight', { value: 960 })
    fireEvent.load(bg)
  }

  it('with no background selected, clicking opens the picker instead', async () => {
    setup([ready])
    await userEvent.click(await screen.findByRole('button', { name: /auto-align/i }))
    expect(await screen.findByAltText(/startrails thumbnail 2026-08-16/i)).toBeInTheDocument()
  })

  it('detects, solves, applies to the draft and reports', async () => {
    // (700, 214) sits on the circle the pole traces around the optical center
    // for this fixture's zenith calibration (lat 50.45°, f 1.48mm/3.75µm on a
    // 1280×960 frame) — the only pixels a zenith-mode roll-only solve can
    // actually reach; see the geometry check in the task-7 report.
    const detectPole = vi.fn().mockResolvedValue({ poleXPx: 700, poleYPx: 214, confidence: 0.87 })
    setup([ready], { detectPole })
    await selectNight()
    await userEvent.click(screen.getByRole('button', { name: /auto-align/i }))
    expect(detectPole).toHaveBeenCalledWith('2026-08-16')
    expect(await screen.findByText(/pole at 700, 214 · confidence 87%/i)).toBeInTheDocument()
  })

  it('flags low confidence as likely unreliable', async () => {
    const detectPole = vi.fn().mockResolvedValue({ poleXPx: 700, poleYPx: 214, confidence: 0.2 })
    setup([ready], { detectPole })
    await selectNight()
    await userEvent.click(screen.getByRole('button', { name: /auto-align/i }))
    expect(await screen.findByText(/likely unreliable/i)).toBeInTheDocument()
  })

  it('refuses a mismatched startrails and reports API errors', async () => {
    const detectPole = vi.fn().mockRejectedValue(new Error('HTTP 422: no'))
    setup([ready], { detectPole })
    // mismatch: load as 720×720
    await userEvent.click(await screen.findByRole('button', { name: /^startrails/i }))
    await userEvent.click(await screen.findByText('2026-08-16'))
    const bg = screen.getByAltText('Startrails 2026-08-16')
    Object.defineProperty(bg, 'naturalWidth', { value: 720 })
    Object.defineProperty(bg, 'naturalHeight', { value: 720 })
    fireEvent.load(bg)
    await userEvent.click(screen.getByRole('button', { name: /auto-align/i }))
    expect(await screen.findByText(/can.t map its coordinates/i)).toBeInTheDocument()
    expect(detectPole).not.toHaveBeenCalled()
  })
})
