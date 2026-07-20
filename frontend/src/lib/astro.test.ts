import { describe, it, expect } from 'vitest'
import {
  altAzToImage, altitudeOf, gmstDeg, julianDate, moonEquatorial,
  moonIllumination, raDecToAltAz, sunEquatorial,
} from './astro'

const cal = { cx: 480, cy: 480, radiusPx: 440, rotationDeg: 0, flip: false }

describe('astro', () => {
  it('computes JD and GMST at the J2000 epoch', () => {
    const jd = julianDate(new Date(Date.UTC(2000, 0, 1, 12, 0, 0)))
    expect(jd).toBeCloseTo(2451545.0, 6)
    expect(gmstDeg(jd)).toBeCloseTo(280.4606, 3)
  })

  it('object at dec=lat crossing the meridian is at the zenith', () => {
    const { altDeg } = raDecToAltAz(120, 50, 50, 120) // HA = 0
    expect(altDeg).toBeCloseTo(90, 5)
  })

  it('celestial pole sits at alt=lat, az=0 for any LST', () => {
    for (const lst of [0, 90, 217]) {
      const { altDeg, azDeg } = raDecToAltAz(33, 90, 50.45, lst)
      expect(altDeg).toBeCloseTo(50.45, 4)
      expect(Math.min(azDeg, 360 - azDeg)).toBeCloseTo(0, 4)
    }
  })

  it('zenith projects to the lens center regardless of azimuth', () => {
    const p = altAzToImage(90, 123, cal)
    expect(p.x).toBeCloseTo(480)
    expect(p.y).toBeCloseTo(480)
  })

  it('horizon N projects straight up, E straight right', () => {
    const n = altAzToImage(0, 0, cal)
    expect(n.x).toBeCloseTo(480)
    expect(n.y).toBeCloseTo(480 - 440)
    const e = altAzToImage(0, 90, cal)
    expect(e.x).toBeCloseTo(480 + 440)
    expect(e.y).toBeCloseTo(480)
  })

  it('rotationDeg rotates north clockwise on the image', () => {
    const n = altAzToImage(0, 0, { ...cal, rotationDeg: 90 })
    expect(n.x).toBeCloseTo(480 + 440)
    expect(n.y).toBeCloseTo(480)
  })

  it('flip mirrors east-west', () => {
    const e = altAzToImage(0, 90, { ...cal, flip: true })
    expect(e.x).toBeCloseTo(480 - 440)
    expect(e.y).toBeCloseTo(480)
  })
})

describe('sun & moon', () => {
  it('sun declination is ~+23.4° at June solstice and ~0° at March equinox', () => {
    expect(sunEquatorial(new Date(Date.UTC(2026, 5, 21, 12))).decDeg).toBeCloseTo(23.4, 0)
    expect(Math.abs(sunEquatorial(new Date(Date.UTC(2026, 2, 20, 12))).decDeg)).toBeLessThan(1)
  })

  it('sun is high at solar noon and below horizon at solar midnight in Kyiv', () => {
    const noon = new Date(Date.UTC(2026, 5, 21, 10)) // ≈ solar noon for 30.5°E
    const s1 = sunEquatorial(noon)
    expect(altitudeOf(noon, s1.raDeg, s1.decDeg, 50.45, 30.52)).toBeGreaterThan(55)
    const midnight = new Date(Date.UTC(2026, 5, 21, 22))
    const s2 = sunEquatorial(midnight)
    expect(altitudeOf(midnight, s2.raDeg, s2.decDeg, 50.45, 30.52)).toBeLessThan(-5)
  })

  it('moon illumination is ~0% at a known new moon and ~100% at full moon', () => {
    // documented lunations: new 2000-01-06 18:14 UTC, full 2000-01-21 04:40 UTC
    expect(moonIllumination(new Date(Date.UTC(2000, 0, 6, 18, 14))).pct).toBeLessThan(2)
    expect(moonIllumination(new Date(Date.UTC(2000, 0, 21, 4, 40))).pct).toBeGreaterThan(97)
  })

  it('moon waxes between new and full and returns plausible coordinates', () => {
    const mid = new Date(Date.UTC(2000, 0, 14))
    expect(moonIllumination(mid).waxing).toBe(true)
    const m = moonEquatorial(mid)
    expect(m.raDeg).toBeGreaterThanOrEqual(0)
    expect(m.raDeg).toBeLessThan(360)
    expect(Math.abs(m.decDeg)).toBeLessThanOrEqual(29)
  })
})
