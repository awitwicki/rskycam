import { describe, it, expect } from 'vitest'
import type { OverlayGeometry } from '../api/types'
import { altAzToImage, lstDeg, raDecToAltAz } from './astro'
import { buildOverlayGeometry, cropGeometry } from './overlayGeometry'

const base = {
  time: new Date(Date.UTC(2026, 6, 14, 0, 0, 0)),
  location: { latitudeDeg: 50.45, longitudeDeg: 30.52 },
  calibration: { cx: 480, cy: 480, radiusPx: 440, rotationDeg: 0, flip: false },
  imageWidth: 960,
  imageHeight: 960,
}
const none = { cardinal: false, altAzGrid: false, raDecGrid: false }

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
    const pole = altAzToImage(ncp.altDeg, ncp.azDeg, base.calibration)

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
