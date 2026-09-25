import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
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
  image: {
    maskMode: 'none', maskCenterXPx: 640, maskCenterYPx: 480, maskRadiusPx: 620, crop: null,
    meterPolygons: [],
  },
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
  rtsp: {
    enabled: false, port: 8554, fps: 5, overlay: true, outputWidth: 0,
    bitrateKbps: 2000, authEnabled: true, extraArgs: '',
  },
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
  astro: {
    sunAltDeg: -20, moonAltDeg: 10, moonPhasePct: 50, moonWaxing: true,
    sunriseIso: null, sunsetIso: null, astroDuskIso: null, astroDawnIso: null,
    moonriseIso: null, moonsetIso: null, moonTransitIso: '2026-07-14T00:00:00Z',
  },
  camera: null,
  darksProgress: null,
  focus: { enabled: false, exposureUs: 1_000_000, gain: 1 },
  rtsp: {
    enabled: false, listening: false, port: 8554, username: 'admin',
    clients: 0, encoding: false, lastError: null,
  },
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

describe('OverlayEditorPage metering mask', () => {
  it('explains the default, then adds and trims a region', async () => {
    setup([])
    await userEvent.click(await screen.findByRole('button', { name: /metering mask/i }))

    // With no regions the whole frame is metered — say so, rather than
    // showing an empty panel the user has to interpret.
    expect(screen.getByText(/whole frame/i)).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: /add region/i }))
    expect(screen.getByText(/region 1 · 4 pts/i)).toBeInTheDocument()
    expect(screen.queryByText(/whole frame/i)).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: /\+ vertex/i }))
    expect(screen.getByText(/region 1 · 5 pts/i)).toBeInTheDocument()

    // Trimming stops at the 3-point floor rather than destroying the region.
    const minus = screen.getByRole('button', { name: /− vertex/i })
    await userEvent.click(minus)
    await userEvent.click(minus)
    await userEvent.click(minus)
    expect(screen.getByText(/region 1 · 3 pts/i)).toBeInTheDocument()
  })

  it('scopes per-vertex controls to the selected region and clamps selection after delete', async () => {
    setup([])
    await userEvent.click(await screen.findByRole('button', { name: /metering mask/i }))

    await userEvent.click(screen.getByRole('button', { name: /add region/i }))
    await userEvent.click(screen.getByRole('button', { name: /add region/i }))
    expect(screen.getByText(/region 1 · 4 pts/i)).toBeInTheDocument()
    expect(screen.getByText(/region 2 · 4 pts/i)).toBeInTheDocument()

    // Region 1 is selected by default (it was added first). Only the
    // selected region's row may expose +/− vertex controls: the canvas's
    // "bigger dot" cue is drawn only for the selected region, so a button
    // that could act on some other, unshown region's vertex would disagree
    // with what the user is actually shown.
    const region2Row = screen.getByText(/region 2 · 4 pts/i).closest('div')!
    expect(within(region2Row).queryByRole('button', { name: /− vertex/i })).not.toBeInTheDocument()
    expect(within(region2Row).queryByRole('button', { name: /\+ vertex/i })).not.toBeInTheDocument()
    expect(within(region2Row).queryByRole('button', { name: /delete/i })).not.toBeInTheDocument()

    // Give region 1 a different point count than region 2, then switch
    // selection to region 2 and trim it from there. The right region must
    // change (region 1 stays at 5), not whatever region was selected before.
    await userEvent.click(screen.getByRole('button', { name: /\+ vertex/i }))
    expect(screen.getByText(/region 1 · 5 pts/i)).toBeInTheDocument()

    await userEvent.click(screen.getByText(/region 2 · 4 pts/i))
    await userEvent.click(within(region2Row).getByRole('button', { name: /− vertex/i }))
    expect(screen.getByText(/region 1 · 5 pts/i)).toBeInTheDocument() // untouched
    expect(screen.getByText(/region 2 · 3 pts/i)).toBeInTheDocument()

    // Delete the still-selected region 2 — selection must clamp back onto
    // the one remaining region rather than pointing past the end of the
    // (now shorter) array, which would leave no row showing controls.
    await userEvent.click(within(region2Row).getByRole('button', { name: /delete/i }))
    expect(screen.queryByText(/region 2/i)).not.toBeInTheDocument()
    const region1Row = screen.getByText(/region 1 · 5 pts/i).closest('div')!
    expect(within(region1Row).getByRole('button', { name: /\+ vertex/i })).toBeInTheDocument()

    // Deleting the last remaining region returns to the empty state.
    await userEvent.click(within(region1Row).getByRole('button', { name: /delete/i }))
    expect(screen.getByText(/whole frame/i)).toBeInTheDocument()
  })

  it('keeps the enlarged vertex dot valid after − vertex, so the next click removes the shown one', async () => {
    // jsdom has neither of these; the editor canvas calls both on pointer
    // down/up during a real drag.
    if (!HTMLElement.prototype.setPointerCapture) {
      HTMLElement.prototype.setPointerCapture = () => {}
      HTMLElement.prototype.releasePointerCapture = () => {}
    }
    const rect = {
      left: 0, top: 0, width: 1280, height: 960, right: 1280, bottom: 960, x: 0, y: 0,
      toJSON: () => ({}),
    } as DOMRect
    vi.spyOn(HTMLCanvasElement.prototype, 'getBoundingClientRect').mockReturnValue(rect)
    // The stubbed 2D context in src/test/setup.ts is a single shared object
    // returned by every canvas.getContext('2d') call, so spying on its arc()
    // catches every dot drawMeterRegions draws, across every re-render.
    const ctx = document.createElement('canvas').getContext('2d')!
    const arcSpy = vi.spyOn(ctx, 'arc')
    const lastEnlarged = () => {
      const calls = arcSpy.mock.calls.filter((c) => c[2] === 9) // radius 9 = "chosen" dot
      return calls.length ? calls[calls.length - 1] : null
    }

    setup([])
    await userEvent.click(await screen.findByRole('button', { name: /metering mask/i }))
    await userEvent.click(screen.getByRole('button', { name: /add region/i }))
    // Quad (0.25,0.25) (0.75,0.25) (0.75,0.75) (0.25,0.75) — all edges equal
    // length, so the longest-edge split picks the first edge and inserts its
    // midpoint after index 0: (0.25,0.25) (0.5,0.25) (0.75,0.25) (0.75,0.75)
    // (0.25,0.75). Index 4, the region's last point, is (0.25, 0.75).
    await userEvent.click(screen.getByRole('button', { name: /\+ vertex/i }))
    expect(screen.getByText(/region 1 · 5 pts/i)).toBeInTheDocument()

    const canvas = document.querySelector('canvas')!
    fireEvent.pointerDown(canvas, { clientX: 0.25 * 1280, clientY: 0.75 * 960, pointerId: 1 })
    fireEvent.pointerUp(canvas, { pointerId: 1 })
    expect(lastEnlarged()).toEqual([320, 720, 9, 0, Math.PI * 2])

    const minus = screen.getByRole('button', { name: /− vertex/i })
    await userEvent.click(minus)
    expect(screen.getByText(/region 1 · 4 pts/i)).toBeInTheDocument()
    // Index 4 is gone (only 0..3 remain) — the selection must have been
    // reset to a valid index (3, the new last point at (0.75,0.75) = pixel
    // (960,720)), not left dangling at 4 with nothing drawn enlarged.
    expect(lastEnlarged()).toEqual([960, 720, 9, 0, Math.PI * 2])

    const callsBeforeSecondClick = arcSpy.mock.calls.length
    await userEvent.click(minus)
    expect(screen.getByText(/region 1 · 3 pts/i)).toBeInTheDocument()
    // The second click must remove the vertex that was actually shown
    // enlarged — (0.75,0.75) — not some other, unindicated one. Only the
    // dots drawn by THIS click's render matter here: earlier renders drew a
    // (non-enlarged) dot at that same pixel too, back when it was a
    // still-present vertex.
    const drawnThisRender = arcSpy.mock.calls.slice(callsBeforeSecondClick)
    expect(drawnThisRender.some((c) => c[0] === 960 && c[1] === 720)).toBe(false)
    // And the invariant holds again: a new, still-valid vertex is enlarged.
    expect(lastEnlarged()).toEqual([960, 240, 9, 0, Math.PI * 2])
  })
})
