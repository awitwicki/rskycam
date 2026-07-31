import type { LensCalibration, LensType } from '../api/types'

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

/** Frame dimensions plus the camera's native sensor width, for plate scale. */
export interface LensView {
  frameWidth: number
  frameHeight: number
  /** Native sensor width in px (CameraCaps.maxWidth); == frameWidth when unknown. */
  nativeWidth: number
}

/** Focal length in pixels at this frame's resolution. */
export function focalLengthPx(cal: LensCalibration, view: LensView): number {
  const binning = view.nativeWidth / view.frameWidth
  return (cal.focalLengthMm * 1000) / (cal.pixelSizeUm * binning)
}

/** Furthest usable angle from the optical axis, per lens type. */
export function thetaMaxDeg(lens: LensType): number {
  return lens === 'fisheye' ? 120 : 85
}

export function opticalCenter(cal: LensCalibration, view: LensView): { x: number; y: number } {
  return {
    x: view.frameWidth / 2 + cal.centerOffsetXPx,
    y: view.frameHeight / 2 + cal.centerOffsetYPx,
  }
}

export type Vec3 = [number, number, number]

const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

/** Camera basis in ENU (x=east, y=north, z=up): at pointing alt 90 / az 0 /
 *  roll 0 the image is north-up east-right — the legacy zenith model — and
 *  +roll rotates the sky clockwise. */
export function camBasis(cal: LensCalibration): { fwd: Vec3; right: Vec3; up: Vec3 } {
  const saz = Math.sin(cal.pointingAzDeg * DEG)
  const caz = Math.cos(cal.pointingAzDeg * DEG)
  const salt = Math.sin(cal.pointingAltDeg * DEG)
  const calt = Math.cos(cal.pointingAltDeg * DEG)
  const fwd: Vec3 = [calt * saz, calt * caz, salt]
  const u0: Vec3 = [salt * saz, salt * caz, -calt]
  const r0: Vec3 = [caz, -saz, 0]
  const sr = Math.sin(cal.rollDeg * DEG)
  const cr = Math.cos(cal.rollDeg * DEG)
  const right: Vec3 = [cr * r0[0] + sr * u0[0], cr * r0[1] + sr * u0[1], cr * r0[2] + sr * u0[2]]
  const up: Vec3 = [cr * u0[0] - sr * r0[0], cr * u0[1] - sr * r0[1], cr * u0[2] - sr * r0[2]]
  return { fwd, right, up }
}

/** r = f·tan θ diverges at 90°; clamp so culled points still get finite pixels. */
const RECTILINEAR_THETA_CLAMP_DEG = 89.5

/** Pixel distance from the optical center at `thetaDeg` from the optical
 *  axis — the lens's radial mapping (fisheye r = f·θ, rectilinear r = f·tan θ). */
export function thetaToRadiusPx(cal: LensCalibration, view: LensView, thetaDeg: number): number {
  const fPx = focalLengthPx(cal, view)
  const theta = thetaDeg * DEG
  return cal.lensType === 'fisheye'
    ? fPx * theta
    : fPx * Math.tan(Math.min(theta, RECTILINEAR_THETA_CLAMP_DEG * DEG))
}

/** Physical lens projection into source-image pixels. */
export function altAzToImage(
  altDeg: number, azDeg: number, cal: LensCalibration, view: LensView,
): { x: number; y: number; thetaDeg: number } {
  const a = altDeg * DEG
  const z = azDeg * DEG
  const v: Vec3 = [Math.cos(a) * Math.sin(z), Math.cos(a) * Math.cos(z), Math.sin(a)]
  const { fwd, right, up } = camBasis(cal)
  const theta = Math.acos(Math.min(1, Math.max(-1, dot(fwd, v))))
  const r = thetaToRadiusPx(cal, view, theta / DEG)
  const phi = Math.atan2(dot(v, right), dot(v, up))
  const sx = cal.flip ? -1 : 1
  const oc = opticalCenter(cal, view)
  return { x: oc.x + sx * r * Math.sin(phi), y: oc.y - r * Math.cos(phi), thetaDeg: theta / DEG }
}
