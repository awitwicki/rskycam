import { describe, it, expect } from 'vitest'
import {
  altAzToImage, altitudeOf, gmstDeg, julianDate, moonEquatorial,
  moonIllumination, raDecToAltAz, sunEquatorial, thetaToRadiusPx,
} from './astro'
import type { LensCalibration } from '../api/types'

const cal: LensCalibration = {
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
const view = { frameWidth: 960, frameHeight: 960, nativeWidth: 960 }

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
    const p = altAzToImage(90, 123, cal, view)
    expect(p.x).toBeCloseTo(480)
    expect(p.y).toBeCloseTo(480)
  })

  it('horizon N projects straight up, E straight right (legacy vectors)', () => {
    const n = altAzToImage(0, 0, cal, view)
    expect(n.x).toBeCloseTo(480)
    expect(n.y).toBeCloseTo(40)
    expect(n.thetaDeg).toBeCloseTo(90)
    const e = altAzToImage(0, 90, cal, view)
    expect(e.x).toBeCloseTo(920)
    expect(e.y).toBeCloseTo(480)
  })

  it('rollDeg rotates north clockwise on the image', () => {
    const n = altAzToImage(0, 0, { ...cal, rollDeg: 90 }, view)
    expect(n.x).toBeCloseTo(920)
    expect(n.y).toBeCloseTo(480)
  })

  it('flip mirrors east-west', () => {
    const e = altAzToImage(0, 90, { ...cal, flip: true }, view)
    expect(e.x).toBeCloseTo(40)
    expect(e.y).toBeCloseTo(480)
  })

  it('tilted pointing: pointing → center, zenith lands fPx·π/4 below', () => {
    const c = { ...cal, pointingAzDeg: 180, pointingAltDeg: 45 }
    const p = altAzToImage(45, 180, c, view)
    expect(p.x).toBeCloseTo(480)
    expect(p.y).toBeCloseTo(480)
    const z = altAzToImage(90, 0, c, view)
    expect(z.x).toBeCloseTo(480)
    expect(z.y).toBeCloseTo(700)
    expect(z.thetaDeg).toBeCloseTo(45)
  })

  it('rectilinear projects r = f·tan θ', () => {
    const p = altAzToImage(45, 0, { ...cal, lensType: 'rectilinear' as const }, view)
    expect(p.x).toBeCloseTo(480)
    expect(p.y).toBeCloseTo(480 - 880 / Math.PI)
  })

  it('thetaToRadiusPx matches the lens mapping (Rust parity)', () => {
    expect(thetaToRadiusPx(cal, view, 90)).toBeCloseTo(440, 6)
    expect(thetaToRadiusPx(cal, view, 45)).toBeCloseTo(220, 6)
    const rect = { ...cal, lensType: 'rectilinear' as const }
    expect(thetaToRadiusPx(rect, view, 45)).toBeCloseTo(880 / Math.PI, 6)
    expect(Number.isFinite(thetaToRadiusPx(rect, view, 90))).toBe(true)
  })

  it('binning: half-resolution frame halves the plate scale', () => {
    const v = { frameWidth: 480, frameHeight: 480, nativeWidth: 960 }
    const n = altAzToImage(0, 0, cal, v)
    expect(n.x).toBeCloseTo(240)
    expect(n.y).toBeCloseTo(20)
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
