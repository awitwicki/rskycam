import type { LensCalibration } from '../api/types'

const DEG = Math.PI / 180

export function julianDate(d: Date): number {
  return d.getTime() / 86_400_000 + 2_440_587.5
}

/** Greenwich mean sidereal time in degrees, [0, 360). */
export function gmstDeg(jd: number): number {
  const gmst = 280.46061837 + 360.98564736629 * (jd - 2451545.0)
  return ((gmst % 360) + 360) % 360
}

/** Local sidereal time in degrees; east longitude positive. */
export function lstDeg(d: Date, lonDeg: number): number {
  return (((gmstDeg(julianDate(d)) + lonDeg) % 360) + 360) % 360
}

/** Azimuth measured from north, clockwise (east = 90°). */
export function raDecToAltAz(
  raDeg: number, decDeg: number, latDeg: number, lstDegVal: number,
): { altDeg: number; azDeg: number } {
  const ha = (lstDegVal - raDeg) * DEG
  const dec = decDeg * DEG
  const lat = latDeg * DEG
  const sinAlt = Math.sin(dec) * Math.sin(lat) + Math.cos(dec) * Math.cos(lat) * Math.cos(ha)
  const alt = Math.asin(Math.min(1, Math.max(-1, sinAlt)))
  const az = Math.atan2(
    -Math.cos(dec) * Math.sin(ha),
    Math.sin(dec) * Math.cos(lat) - Math.cos(dec) * Math.sin(lat) * Math.cos(ha),
  )
  return { altDeg: alt / DEG, azDeg: (((az / DEG) % 360) + 360) % 360 }
}

function norm360(x: number): number {
  return ((x % 360) + 360) % 360
}

function obliquityRad(n: number): number {
  return (23.439 - 0.0000004 * n) * DEG
}

/** Low-precision solar ecliptic longitude (±0.01°), n = days since J2000. */
function sunEclipticLonDeg(n: number): number {
  const L = 280.46 + 0.9856474 * n
  const g = (357.528 + 0.9856003 * n) * DEG
  return norm360(L + 1.915 * Math.sin(g) + 0.02 * Math.sin(2 * g))
}

/** Low-precision lunar ecliptic coordinates (~1° accuracy). */
function moonEcliptic(n: number): { lonDeg: number; latDeg: number } {
  const L = 218.316 + 13.176396 * n
  const M = (134.963 + 13.064993 * n) * DEG
  const F = (93.272 + 13.22935 * n) * DEG
  return {
    lonDeg: norm360(L + 6.289 * Math.sin(M)),
    latDeg: 5.128 * Math.sin(F),
  }
}

function eclipticToEquatorial(
  lonDeg: number, latDeg: number, n: number,
): { raDeg: number; decDeg: number } {
  const lam = lonDeg * DEG
  const beta = latDeg * DEG
  const eps = obliquityRad(n)
  const raDeg = Math.atan2(
    Math.sin(lam) * Math.cos(eps) - Math.tan(beta) * Math.sin(eps),
    Math.cos(lam),
  ) / DEG
  const decDeg = Math.asin(
    Math.sin(beta) * Math.cos(eps) + Math.cos(beta) * Math.sin(eps) * Math.sin(lam),
  ) / DEG
  return { raDeg: norm360(raDeg), decDeg }
}

export function sunEquatorial(d: Date): { raDeg: number; decDeg: number } {
  const n = julianDate(d) - 2451545.0
  return eclipticToEquatorial(sunEclipticLonDeg(n), 0, n)
}

export function moonEquatorial(d: Date): { raDeg: number; decDeg: number } {
  const n = julianDate(d) - 2451545.0
  const { lonDeg, latDeg } = moonEcliptic(n)
  return eclipticToEquatorial(lonDeg, latDeg, n)
}

/** Altitude of a body with fixed equatorial coordinates at a given time/place. */
export function altitudeOf(
  d: Date, raDeg: number, decDeg: number, latDeg: number, lonDeg: number,
): number {
  return raDecToAltAz(raDeg, decDeg, latDeg, lstDeg(d, lonDeg)).altDeg
}

/** Illuminated fraction of the Moon (0–100) and whether it is waxing. */
export function moonIllumination(d: Date): { pct: number; waxing: boolean } {
  const n = julianDate(d) - 2451545.0
  const elong = norm360(moonEcliptic(n).lonDeg - sunEclipticLonDeg(n))
  return {
    pct: ((1 - Math.cos(elong * DEG)) / 2) * 100,
    waxing: elong < 180,
  }
}

/** Equidistant fisheye projection into source-image pixels. */
export function altAzToImage(
  altDeg: number, azDeg: number, cal: LensCalibration,
): { x: number; y: number } {
  const r = (cal.radiusPx * (90 - altDeg)) / 90
  const theta = (azDeg + cal.rotationDeg) * DEG
  const sx = cal.flip ? -1 : 1
  return { x: cal.cx + sx * r * Math.sin(theta), y: cal.cy - r * Math.cos(theta) }
}
