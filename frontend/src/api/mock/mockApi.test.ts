import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ApiEvent } from '../types'
import { MockApi } from './mockApi'

const stub = () => new MockApi({ renderFrame: () => 'data:image/jpeg;base64,x' })

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
  vi.useRealTimers()
})

describe('MockApi auth', () => {
  it('rejects wrong credentials', async () => {
    const api = stub()
    expect(await api.login('admin', 'nope')).toBe(false)
    expect(api.isAuthenticated()).toBe(false)
  })

  it('accepts default credentials and logs out', async () => {
    const api = stub()
    expect(await api.login('admin', 'pa$$word!0')).toBe(true)
    expect(api.isAuthenticated()).toBe(true)
    await api.logout()
    expect(api.isAuthenticated()).toBe(false)
  })

  it('changePassword requires the old password and persists', async () => {
    const api = stub()
    expect(await api.changePassword('wrong', 'new')).toBe(false)
    expect(await api.changePassword('pa$$word!0', 'hunter2')).toBe(true)
    expect(await api.login('admin', 'pa$$word!0')).toBe(false)
    expect(await api.login('admin', 'hunter2')).toBe(true)
  })
})

describe('MockApi status & events', () => {
  it('returns a plausible status', async () => {
    const s = await stub().getStatus()
    expect(s.capture.state).toBe('capturing')
    expect(s.system.ramTotalMb).toBeGreaterThan(0)
    expect(s.system.undervoltageSinceBoot).toBe(true)
    expect(s.sensor.state).toBe('ok')
    expect(s.sensor.reading?.temperatureC).toBeTypeOf('number')
  })

  it('reports the sensor as disabled when turned off in settings', async () => {
    const api = stub()
    const s = await api.getSettings()
    s.sensor.enabled = false
    await api.putSettings(s)
    expect((await api.getStatus()).sensor).toEqual({ state: 'disabled', reading: null })
  })

  it('subscribe emits immediately and on intervals; unsubscribe stops', async () => {
    vi.useFakeTimers()
    const api = stub()
    const events: ApiEvent[] = []
    const un = api.subscribe((e) => events.push(e))
    await vi.advanceTimersByTimeAsync(0)
    expect(events.some((e) => e.type === 'frame')).toBe(true)
    expect(events.some((e) => e.type === 'status')).toBe(true)
    const before = events.length
    await vi.advanceTimersByTimeAsync(5100)
    expect(events.length).toBeGreaterThan(before)
    un()
    const after = events.length
    await vi.advanceTimersByTimeAsync(10000)
    expect(events.length).toBe(after)
  })
})

describe('MockApi settings & overlay', () => {
  it('settings roundtrip persists', async () => {
    const api = stub()
    const s = await api.getSettings()
    s.location.latitudeDeg = 48.85
    await api.putSettings(s)
    expect((await api.getSettings()).location.latitudeDeg).toBe(48.85)
  })

  it('overlay honors calibration override and appends text-field labels', async () => {
    const api = stub()
    const g = await api.getOverlay({
      calibration: { cx: 100, cy: 100, radiusPx: 90, rotationDeg: 0, flip: false },
    })
    expect(g.polylines.length).toBeGreaterThan(0)
    const textLabels = g.labels.filter((l) => l.layer === 'text')
    expect(textLabels.length).toBeGreaterThan(0)
    expect(textLabels[0].align).toBe('left')
  })

  it('reports astro status and a 24h lightgraph', async () => {
    const api = stub()
    const status = await api.getStatus()
    expect(status.astro.sunAltDeg).toBeGreaterThan(-90)
    expect(status.astro.sunAltDeg).toBeLessThan(90)
    expect(status.astro.moonPhasePct).toBeGreaterThanOrEqual(0)
    expect(status.astro.moonPhasePct).toBeLessThanOrEqual(100)
    const lg = await api.getLightgraph()
    expect(lg.sunAltDeg).toHaveLength(144)
    expect(lg.stepMinutes).toBe(10)
    // over a full day the sun is both up and well below the horizon
    expect(Math.max(...lg.sunAltDeg)).toBeGreaterThan(0)
    expect(Math.min(...lg.sunAltDeg)).toBeLessThan(-10)
  })

  it('fills newly added settings sections when older settings are stored', async () => {
    localStorage.setItem(
      'rskycam.mock.settings',
      JSON.stringify({
        location: { latitudeDeg: 1, longitudeDeg: 2 },
        // an overlay section from before gridOpacity existed
        overlay: { calibration: { cx: 11, cy: 22, radiusPx: 33, rotationDeg: 0, flip: false } },
      }),
    )
    const s = await stub().getSettings()
    expect(s.image.maskMode).toBeDefined()
    expect(s.camera.driver).toBe('mock')
    expect(s.location.latitudeDeg).toBe(1)
    expect(s.overlay.calibration.cx).toBe(11) // stored values win
    expect(s.overlay.gridOpacity).toBe(0.45) // new overlay field gains its default
    expect(s.overlay.layers.cardinal).toBe(true)
  })

  it('grid polylines carry the settings gridOpacity; request override wins', async () => {
    const api = stub()
    const g = await api.getOverlay({})
    expect(g.polylines[0].opacity).toBe(0.45)
    const g2 = await api.getOverlay({ gridOpacity: 0.8 })
    expect(g2.polylines[0].opacity).toBe(0.8)
  })

  it('offsets overlay geometry by the configured crop; crop:null gives sensor space', async () => {
    const api = stub()
    const s = await api.getSettings()
    s.image.crop = { x: 100, y: 50, width: 700, height: 800 }
    await api.putSettings(s)

    const sensor = await api.getOverlay({ crop: null })
    expect(sensor.imageWidth).toBe(1280)
    expect(sensor.imageHeight).toBe(960)

    const cropped = await api.getOverlay({})
    expect(cropped.imageWidth).toBe(700)
    expect(cropped.imageHeight).toBe(800)
    expect(cropped.polylines[0].points[0][0])
      .toBeCloseTo(sensor.polylines[0].points[0][0] - 100)
    expect(cropped.polylines[0].points[0][1])
      .toBeCloseTo(sensor.polylines[0].points[0][1] - 50)
  })
})

describe('MockApi nights', () => {
  it('lists 8 nights, newest first, with all artifact states represented', async () => {
    const nights = await stub().getNights()
    expect(nights).toHaveLength(8)
    expect([...nights.map((n) => n.date)].sort().reverse()).toEqual(nights.map((n) => n.date))
    const states = nights.flatMap((n) => [n.keogram.state, n.startrails.state, n.timelapse.state])
    for (const s of ['ready', 'generating', 'error', 'disabled']) expect(states).toContain(s)
  })

  it('getNight returns frames; unknown date rejects', async () => {
    const api = stub()
    const first = (await api.getNights())[0]
    const detail = await api.getNight(first.date)
    expect(detail.frames.length).toBeGreaterThan(0)
    await expect(api.getNight('1999-01-01')).rejects.toThrow(/not found/)
  })

  it('rebuildNight flips timelapse to generating', async () => {
    const api = stub()
    const nights = await api.getNights()
    const ready = nights.find((n) => n.timelapse.state === 'ready')!
    await api.rebuildNight(ready.date)
    expect((await api.getNight(ready.date)).timelapse.state).toBe('generating')
  })

  it('rebuildNight rejects for an unknown date', async () => {
    await expect(stub().rebuildNight('1999-01-01')).rejects.toThrow(/not found/)
  })
})
