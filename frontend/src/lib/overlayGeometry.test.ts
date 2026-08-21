import { describe, it, expect } from 'vitest'
import type { LensCalibration, OverlayGeometry } from '../api/types'
import { altAzToImage, lstDeg, raDecToAltAz } from './astro'
import { buildOverlayGeometry, cropGeometry } from './overlayGeometry'

const calibration: LensCalibration = {
  lensType: 'fisheye' as const,
  focalLengthMm: 0.88 / Math.PI, // fPx = 880/π → horizon at 440 px
  pixelSizeUm: 1,
  pointingAzDeg: 0,
  pointingAltDeg: 90,
  rollDeg: 0,
  flip: false,
  centerOffsetXPx: 0,
  centerOffsetYPx: 0,
}

const base = {
  time: new Date(Date.UTC(2026, 6, 14, 0, 0, 0)),
  location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
  calibration,
  imageWidth: 960,
  imageHeight: 960,
}
const none = { cardinal: false, altAzGrid: false, raDecGrid: false, constellations: false }

describe('buildOverlayGeometry', () => {
  it('returns nothing when all layers are off', () => {
    const g = buildOverlayGeometry({ ...base, layers: none })
    expect(g.polylines).toHaveLength(0)
    expect(g.labels).toHaveLength(0)
    expect(g.imageWidth).toBe(960)
  })

  it('altAz grid: 3 altitude circles + 8 azimuth radials, all tagged altAz', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, altAzGrid: true } })
    expect(g.polylines).toHaveLength(11)
    expect(g.polylines.every((p) => p.layer === 'altAz')).toBe(true)
  })

  it('horizon circle points sit at radiusPx from center', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, altAzGrid: true } })
    const horizon = g.polylines[0] // first circle is alt=0
    for (const [x, y] of horizon.points) {
      expect(Math.hypot(x - 480, y - 480)).toBeCloseTo(440, 6)
    }
  })

  it('cardinal layer emits N/E/S/W labels, N above center', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, cardinal: true } })
    expect(g.labels.map((l) => l.text).sort()).toEqual(['E', 'N', 'S', 'W'])
    const n = g.labels.find((l) => l.text === 'N')!
    expect(n.y).toBeLessThan(480)
    expect(n.x).toBeCloseTo(480, 6)
  })

  it('raDec lines exist and never leave the horizon circle', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, raDecGrid: true } })
    expect(g.polylines.length).toBeGreaterThan(4)
    for (const pl of g.polylines) {
      expect(pl.layer).toBe('raDec')
      expect(pl.points.length).toBeGreaterThan(1)
      for (const [x, y] of pl.points) {
        expect(Math.hypot(x - 480, y - 480)).toBeLessThanOrEqual(440.01)
      }
    }
  })

  it('raDec meridians converge at the celestial pole and a dec 80 circle rings it', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, raDecGrid: true } })
    const lst = lstDeg(base.time, base.location.longitudeDeg)
    const ncp = raDecToAltAz(0, 90, base.location.latitudeDeg, lst)
    const pole = altAzToImage(ncp.altDeg, ncp.azDeg, base.calibration, { frameWidth: 960, frameHeight: 960, nativeWidth: 960 })

    const meridiansAtPole = g.polylines.filter((pl) =>
      pl.points.some(([x, y]) => Math.hypot(x - pole.x, y - pole.y) < 0.01))
    expect(meridiansAtPole.length).toBeGreaterThanOrEqual(12) // every 30° of RA

    // dec 80 → 10° from the pole → r = radius·10/90; a full 121-point circle
    const ring = g.polylines.find((pl) =>
      pl.points.length === 121 &&
      pl.points.every(([x, y]) => Math.hypot(x - pole.x, y - pole.y) < 0.13 * 440))
    expect(ring).toBeDefined()
  })

  it('stamps gridOpacity onto altAz and raDec polylines', () => {
    const g = buildOverlayGeometry({
      ...base, layers: { ...none, altAzGrid: true, raDecGrid: true }, gridOpacity: 0.3,
    })
    expect(g.polylines.length).toBeGreaterThan(0)
    expect(g.polylines.every((p) => p.opacity === 0.3)).toBe(true)
  })

  it('stamps constellationsOpacity onto constellation polylines but not their labels', () => {
    const g = buildOverlayGeometry({
      ...base, layers: { ...none, constellations: true }, constellationsOpacity: 0.2,
    })
    expect(g.polylines.length).toBeGreaterThan(0)
    expect(g.polylines.every((p) => p.layer === 'constellations' && p.opacity === 0.2)).toBe(true)
    expect(g.labels.length).toBeGreaterThan(0)
    expect(g.labels.every((l) => l.layer === 'constellationLabels')).toBe(true)
  })

  it('mask circle culls grid points outside it, but not cardinal labels', () => {
    const layers = { ...none, altAzGrid: true, raDecGrid: true, cardinal: true }
    const mask = { centerXPx: 500, centerYPx: 460, radiusPx: 200 }
    const dist = (x: number, y: number) => Math.hypot(x - mask.centerXPx, y - mask.centerYPx)

    const unmasked = buildOverlayGeometry({ ...base, layers })
    expect(unmasked.polylines.some((pl) => pl.points.some(([x, y]) => dist(x, y) > mask.radiusPx)))
      .toBe(true)

    const g = buildOverlayGeometry({ ...base, layers, mask })
    expect(g.polylines.length).toBeGreaterThan(0)
    for (const pl of g.polylines) {
      expect(pl.points.length).toBeGreaterThan(1)
      for (const [x, y] of pl.points) {
        expect(dist(x, y)).toBeLessThanOrEqual(mask.radiusPx + 0.01)
      }
    }
    // cardinal labels are annotations, not sky lines — the mask leaves them
    expect(g.labels.map((l) => l.text).sort()).toEqual(['E', 'N', 'S', 'W'])
  })

  it('rectilinear culls the horizon circle (θ > 85°)', () => {
    const g = buildOverlayGeometry({
      ...base,
      calibration: { ...base.calibration, lensType: 'rectilinear' as const },
      layers: { ...none, altAzGrid: true },
    })
    // alt-0 circle gone; alt 30/60 circles + 8 radials (lowest points culled)
    expect(g.polylines).toHaveLength(10)
  })

  it('constellations layer projects a known constellation (Ursa Minor) and its label', () => {
    const g = buildOverlayGeometry({ ...base, layers: { ...none, constellations: true } })
    const lst = lstDeg(base.time, base.location.longitudeDeg)
    const view = { frameWidth: 960, frameHeight: 960, nativeWidth: 960 }
    const project = (raDeg: number, decDeg: number) => {
      const { altDeg, azDeg } = raDecToAltAz(raDeg, decDeg, base.location.latitudeDeg, lst)
      return altAzToImage(altDeg, azDeg, base.calibration, view)
    }
    // Ursa Minor (UMi) from constellations.json — circumpolar at latitude
    // 50.45 (lowest dec 71.8 > 90 - lat), so every point stays above the
    // horizon and this renders as a single unsplit 8-point polyline.
    const umiLine: [number, number][] = [
      [236.0147, 77.7945], [244.3762, 75.7553], [230.1821, 71.834], [222.6764, 74.1555],
      [236.0147, 77.7945], [251.4927, 82.0373], [263.0542, 86.5865], [37.9545, 89.2641],
    ]
    const expected = umiLine.map(([ra, dec]) => project(ra, dec))
    const match = g.polylines.find((pl) =>
      pl.layer === 'constellations' &&
      pl.points.length === expected.length &&
      pl.points.every(([x, y], i) => Math.hypot(x - expected[i].x, y - expected[i].y) < 1e-6))
    expect(match).toBeDefined()

    const labelExpected = project(226.5, 68) // UMi's labelRaDeg/labelDecDeg
    const label = g.labels.find((l) => l.layer === 'constellationLabels' && l.text === 'Ursa Minor')
    expect(label).toBeDefined()
    expect(label!.x).toBeCloseTo(labelExpected.x, 6)
    expect(label!.y).toBeCloseTo(labelExpected.y, 6)
  })
})

describe('raDec horizon reach', () => {
  it('dec-0 circle reaches the horizon radius at latitude 90', () => {
    // At latitude 90 the celestial equator IS the horizon: the old 2°
    // altitude floor culled the whole dec-0 circle; it must now render at
    // exactly the horizon radius.
    const g = buildOverlayGeometry({
      time: new Date('2026-01-01T00:00:00Z'),
      location: { latitudeDeg: 90, longitudeDeg: 0 },
      calibration: {
        lensType: 'fisheye', focalLengthMm: 0.88 / Math.PI, pixelSizeUm: 1,
        pointingAzDeg: 0, pointingAltDeg: 90, rollDeg: 0, flip: false,
        centerOffsetXPx: 0, centerOffsetYPx: 0,
      },
      layers: { cardinal: false, altAzGrid: false, raDecGrid: true, constellations: false },
      imageWidth: 1280, imageHeight: 960,
    })
    const radii = g.polylines.flatMap((p) => p.points.map(([x, y]) => Math.hypot(x - 640, y - 480)))
    expect(Math.max(...radii)).toBeCloseTo(440, 0)
  })
})

describe('cropGeometry', () => {
  it('offsets points and labels into crop space and takes the crop dimensions', () => {
    const g: OverlayGeometry = {
      imageWidth: 1280,
      imageHeight: 960,
      polylines: [{ layer: 'altAz', points: [[200, 150], [300, 250]], opacity: 0.3 }],
      labels: [{ layer: 'cardinal', text: 'N', x: 640, y: 30, fontSize: 28 }],
    }
    const c = cropGeometry(g, { x: 100, y: 50, width: 800, height: 700 })
    expect(c.imageWidth).toBe(800)
    expect(c.imageHeight).toBe(700)
    expect(c.polylines[0].points).toEqual([[100, 100], [200, 200]])
    expect(c.polylines[0].layer).toBe('altAz')
    expect(c.polylines[0].opacity).toBe(0.3)
    expect(c.labels[0].x).toBe(540)
    expect(c.labels[0].y).toBe(-20)
    // input is not mutated
    expect(g.polylines[0].points[0]).toEqual([200, 150])
    expect(g.labels[0].x).toBe(640)
  })
})
