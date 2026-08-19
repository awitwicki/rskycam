import type { CropRect, ImageSettings, LensCalibration } from '../api/types'
import {
  altAzToImage, camBasis, focalLengthPx, opticalCenter, thetaMaxDeg, type LensView, type Vec3,
} from './astro'

export type CalibrationTarget = 'roll' | 'pan'
export type CropHandle = 'tl' | 'br'
export type MaskHandle = 'maskCenter' | 'maskRadius'

const MIN_CROP_PX = 100

const DEG = Math.PI / 180

/** Roll handle: fixed-radius marker at image angle rollDeg from image-up. */
export function rollHandlePosition(cal: LensCalibration, view: LensView) {
  const R = 0.35 * Math.min(view.frameWidth, view.frameHeight)
  const oc = opticalCenter(cal, view)
  const sx = cal.flip ? -1 : 1
  const a = cal.rollDeg * DEG
  return { x: oc.x + sx * R * Math.sin(a), y: oc.y - R * Math.cos(a) }
}

export function calibrationHitTest(
  x: number, y: number, cal: LensCalibration, view: LensView, tolPx = 24,
): 'roll' | null {
  const h = rollHandlePosition(cal, view)
  return Math.hypot(x - h.x, y - h.y) <= tolPx ? 'roll' : null
}

export function applyRollDrag(
  x: number, y: number, cal: LensCalibration, view: LensView,
): LensCalibration {
  const oc = opticalCenter(cal, view)
  const sx = cal.flip ? -1 : 1
  const ang = Math.atan2(sx * (x - oc.x), oc.y - y) / DEG
  return { ...cal, rollDeg: (ang + 360) % 360 }
}

/** Inverse of altAzToImage. θ is clamped to the lens's usable field so
 *  points grabbed outside it still pan sanely. */
export function imageToAltAz(
  x: number, y: number, cal: LensCalibration, view: LensView,
): { altDeg: number; azDeg: number } {
  const oc = opticalCenter(cal, view)
  const sx = cal.flip ? -1 : 1
  const dx = (x - oc.x) / sx
  const dy = oc.y - y
  const r = Math.hypot(dx, dy)
  const phi = Math.atan2(dx, dy)
  const fPx = focalLengthPx(cal, view)
  const raw = cal.lensType === 'fisheye' ? r / fPx : Math.atan(r / fPx)
  const theta = Math.min(raw, thetaMaxDeg(cal.lensType) * DEG)
  const { fwd, right, up } = camBasis(cal)
  const st = Math.sin(theta)
  const ct = Math.cos(theta)
  const sp = Math.sin(phi)
  const cp = Math.cos(phi)
  const v: Vec3 = [
    st * (sp * right[0] + cp * up[0]) + ct * fwd[0],
    st * (sp * right[1] + cp * up[1]) + ct * fwd[1],
    st * (sp * right[2] + cp * up[2]) + ct * fwd[2],
  ]
  const altDeg = Math.asin(Math.min(1, Math.max(-1, v[2]))) / DEG
  const azDeg = ((Math.atan2(v[0], v[1]) / DEG) % 360 + 360) % 360
  return { altDeg, azDeg }
}

/** "Grab the sky": keep the alt/az grabbed at pointer-down under the cursor
 *  by shifting the pointing. Call per pointer-move with the current cal. */
export function applySkyPan(
  grab: { altDeg: number; azDeg: number }, x: number, y: number,
  cal: LensCalibration, view: LensView,
): LensCalibration {
  const cur = imageToAltAz(x, y, cal, view)
  const dAz = ((grab.azDeg - cur.azDeg + 540) % 360) - 180
  const az = ((cal.pointingAzDeg + dAz) % 360 + 360) % 360
  const alt = Math.min(90, Math.max(-90, cal.pointingAltDeg + grab.altDeg - cur.altDeg))
  return { ...cal, pointingAzDeg: az, pointingAltDeg: alt }
}

/** "Grab the grid" in zenith (all-sky) mode: move the optical-center offsets
 *  by the pointer delta from the grab point, clamped to the sanitize bounds. */
export function applyCenterPan(
  grab: { offsetX: number; offsetY: number; x: number; y: number },
  x: number, y: number, cal: LensCalibration,
): LensCalibration {
  const clamp = (v: number) => Math.min(5000, Math.max(-5000, v))
  return {
    ...cal,
    centerOffsetXPx: clamp(grab.offsetX + (x - grab.x)),
    centerOffsetYPx: clamp(grab.offsetY + (y - grab.y)),
  }
}

/** Mask circle handles: a center dot and a radius dot on the circle's east edge. */
export function maskHandlePositions(image: ImageSettings) {
  return {
    maskCenter: { x: image.maskCenterXPx, y: image.maskCenterYPx },
    maskRadius: { x: image.maskCenterXPx + image.maskRadiusPx, y: image.maskCenterYPx },
  }
}

export function maskHitTest(
  x: number, y: number, image: ImageSettings, tolPx = 24,
): MaskHandle | null {
  const hp = maskHandlePositions(image)
  for (const h of ['maskRadius', 'maskCenter'] as const) {
    if (Math.hypot(x - hp[h].x, y - hp[h].y) <= tolPx) return h
  }
  return null
}

/** Drag the mask circle by hand: center follows the pointer, radius is the
 *  pointer's distance from the center. Sanitize bounds (±10000 / 20..10000). */
export function applyMaskDrag(
  handle: MaskHandle, x: number, y: number, image: ImageSettings,
): ImageSettings {
  const clamp = (v: number) => Math.min(10_000, Math.max(-10_000, v))
  if (handle === 'maskCenter') {
    return { ...image, maskCenterXPx: clamp(x), maskCenterYPx: clamp(y) }
  }
  const r = Math.hypot(x - image.maskCenterXPx, y - image.maskCenterYPx)
  return { ...image, maskRadiusPx: Math.min(10_000, Math.max(20, r)) }
}

/** Mouse-wheel zoom: scale the effective focal length (sanitize bounds). */
export function applyWheelZoom(cal: LensCalibration, deltaY: number): LensCalibration {
  const f = cal.focalLengthMm * Math.exp(-deltaY * 0.001)
  return { ...cal, focalLengthMm: Math.min(100, Math.max(0.1, f)) }
}

export interface TextFieldBox {
  id: string
  x: number
  y: number
  fontSize: number
  width: number
}

/** Hit-test text fields drawn left-aligned with a middle baseline. */
export function textFieldHitTest(
  px: number, py: number, boxes: TextFieldBox[], padPx = 6,
): string | null {
  for (const b of boxes) {
    if (
      px >= b.x - padPx && px <= b.x + b.width + padPx &&
      py >= b.y - b.fontSize / 2 - padPx && py <= b.y + b.fontSize / 2 + padPx
    ) return b.id
  }
  return null
}

export function cropHandlePositions(c: CropRect) {
  return {
    tl: { x: c.x, y: c.y },
    br: { x: c.x + c.width, y: c.y + c.height },
  }
}

export function cropHitTest(
  x: number, y: number, c: CropRect, tolPx = 24,
): CropHandle | null {
  const hp = cropHandlePositions(c)
  for (const h of ['tl', 'br'] as const) {
    if (Math.hypot(x - hp[h].x, y - hp[h].y) <= tolPx) return h
  }
  return null
}

/** Drag a crop corner; the opposite corner stays fixed. Clamped to the
 *  sensor bounds and a minimum crop size. */
export function applyCropDrag(
  handle: CropHandle, x: number, y: number, c: CropRect,
  boundsW: number, boundsH: number,
): CropRect {
  const px = Math.min(Math.max(x, 0), boundsW)
  const py = Math.min(Math.max(y, 0), boundsH)
  if (handle === 'tl') {
    const nx = Math.min(px, c.x + c.width - MIN_CROP_PX)
    const ny = Math.min(py, c.y + c.height - MIN_CROP_PX)
    return { x: nx, y: ny, width: c.x + c.width - nx, height: c.y + c.height - ny }
  }
  return {
    x: c.x,
    y: c.y,
    width: Math.max(px - c.x, MIN_CROP_PX),
    height: Math.max(py - c.y, MIN_CROP_PX),
  }
}

/** The celestial pole's sky position for a site latitude. */
export function polePosition(latitudeDeg: number): { altDeg: number; azDeg: number } {
  return { altDeg: Math.abs(latitudeDeg), azDeg: latitudeDeg >= 0 ? 0 : 180 }
}

/** Tilted mode's primary solve path: a damped fixed-point walk on the
 *  sky-space mismatch (camera level, roll 180) starting from `cal`'s own
 *  pointing. Cheap, and empirically exact whenever the walk doesn't need to
 *  cross the pointingAltDeg ±90 boundary. */
function fixedPointTiltedAim(
  poleXPx: number, poleYPx: number, cal: LensCalibration, view: LensView,
  pole: { altDeg: number; azDeg: number },
): LensCalibration {
  let best: LensCalibration = { ...cal, rollDeg: 180 }
  for (let i = 0; i < 200; i++) {
    const seen = imageToAltAz(poleXPx, poleYPx, best, view)
    const dAz = ((pole.azDeg - seen.azDeg + 540) % 360) - 180
    const dAlt = pole.altDeg - seen.altDeg
    best = {
      ...best,
      pointingAzDeg: ((best.pointingAzDeg + dAz * 0.7) % 360 + 360) % 360,
      pointingAltDeg: Math.min(90, Math.max(-90, best.pointingAltDeg + dAlt * 0.7)),
    }
    if (Math.abs(dAz) < 1e-7 && Math.abs(dAlt) < 1e-7) break
  }
  return best
}

/** Tilted mode's fallback solve path: a clamped, line-searched Newton step
 *  on the pixel residual with a numeric 2x2 Jacobian, also from `cal`'s own
 *  pointing. Unlike the fixed-point walk, this treats pointingAzDeg and
 *  pointingAltDeg as coupled — the coupling the fixed-point walk's
 *  coordinate-decoupled linearization misses when the target sits near the
 *  lens's field-of-view edge, where that walk's true root is numerically
 *  unstable (a tiny perturbation off it diverges into the ±90 boundary and
 *  sticks there; extra damping doesn't help since the divergence isn't an
 *  overshoot, it's the linearization's own fixed point being repelling).
 *
 *  A single pixel constrains pointingAzDeg/pointingAltDeg exactly (2
 *  equations, 2 unknowns) but not *uniquely*: an unrelated pointing
 *  direction can, near the field-of-view edge, coincidentally place the
 *  pole at the same pixel too (verified by construction — see
 *  editorMath.test.ts history). The 2px residual gate in `solveAimFromPole`
 *  cannot tell such an alias from the true answer, since by definition an
 *  alias satisfies the pixel constraint exactly as well. What actually keeps
 *  this fallback from landing on a distant alias is that Newton starts its
 *  walk from `cal`'s own pointing rather than from a global search — the
 *  same starting point the fixed-point attempt just used — so it only
 *  explores the basin around a calibration that's already close to correct.
 *  That precondition holds here because the caller (the pole-align UI) always
 *  passes the user's current in-progress calibration, never an arbitrary or
 *  default one; this function does not itself verify closeness. */
function newtonTiltedAim(
  poleXPx: number, poleYPx: number, cal: LensCalibration, view: LensView,
  pole: { altDeg: number; azDeg: number },
): LensCalibration {
  const project = (az: number, alt: number) => {
    const c: LensCalibration = { ...cal, rollDeg: 180, pointingAzDeg: az, pointingAltDeg: alt }
    const p = altAzToImage(pole.altDeg, pole.azDeg, c, view)
    return [p.x - poleXPx, p.y - poleYPx] as const
  }
  let az = cal.pointingAzDeg
  let alt = cal.pointingAltDeg
  const EPS = 1e-4
  for (let i = 0; i < 60; i++) {
    const [rx, ry] = project(az, alt)
    const rNorm = Math.hypot(rx, ry)
    if (rNorm < 1e-7) break
    const [rxAz, ryAz] = project(az + EPS, alt)
    // one-sided so the Jacobian probe never itself crosses the alt boundary
    const altEps = alt + EPS > 90 ? -EPS : EPS
    const [rxAlt, ryAlt] = project(az, alt + altEps)
    const j11 = (rxAz - rx) / EPS
    const j21 = (ryAz - ry) / EPS
    const j12 = (rxAlt - rx) / altEps
    const j22 = (ryAlt - ry) / altEps
    const det = j11 * j22 - j12 * j21
    if (Math.abs(det) < 1e-9) break
    const dAz = (-rx * j22 + ry * j12) / det
    const dAlt = (-j11 * ry + j21 * rx) / det
    let step = 1
    let nextAz = az
    let nextAlt = alt
    for (let k = 0; k < 30; k++) {
      nextAz = az + dAz * step
      nextAlt = Math.min(90, Math.max(-90, alt + dAlt * step))
      const [nrx, nry] = project(nextAz, nextAlt)
      if (Math.hypot(nrx, nry) < rNorm) break
      step *= 0.5
    }
    az = nextAz
    alt = nextAlt
  }
  return { ...cal, rollDeg: 180, pointingAzDeg: ((az % 360) + 360) % 360, pointingAltDeg: alt }
}

/** Aim the calibration so the celestial pole projects onto the given pixel.
 *
 *  Zenith mode solves rollDeg (1-D scan + golden-section).
 *
 *  Tilted mode solves pointing az/alt, tried two ways from the same starting
 *  calibration `cal`: a damped fixed-point walk first (see
 *  `fixedPointTiltedAim`), falling back to a clamped Newton step (see
 *  `newtonTiltedAim`) when the walk doesn't land within tolerance. Both
 *  invert `altAzToImage` purely numerically — never re-derive the
 *  flip/roll conventions by hand. Both also only search near `cal`'s own
 *  pointing rather than globally, which is what keeps the Newton fallback
 *  from landing on a distant-but-otherwise-valid alias pointing direction
 *  (see `newtonTiltedAim`'s doc comment) — callers should pass the user's
 *  current in-progress calibration, not an arbitrary default. Returns null
 *  when neither method's residual is within 2 px from that starting point
 *  — either the pixel can't be reached by the current lens model at all,
 *  or (tilted mode) it's only reachable from a pointing far from `cal`. */
export function solveAimFromPole(
  poleXPx: number, poleYPx: number, cal: LensCalibration, view: LensView,
  latitudeDeg: number, zenithMode: boolean,
): LensCalibration | null {
  const pole = polePosition(latitudeDeg)
  const residual = (c: LensCalibration) => {
    const p = altAzToImage(pole.altDeg, pole.azDeg, c, view)
    return Math.hypot(p.x - poleXPx, p.y - poleYPx)
  }
  if (zenithMode) {
    let best = { ...cal, rollDeg: 0 }
    for (let r = 0; r < 360; r += 2) {
      const c = { ...cal, rollDeg: r }
      if (residual(c) < residual(best)) best = c
    }
    let lo = best.rollDeg - 2
    let hi = best.rollDeg + 2
    for (let i = 0; i < 48; i++) {
      const m1 = lo + (hi - lo) * 0.382
      const m2 = lo + (hi - lo) * 0.618
      if (residual({ ...cal, rollDeg: (m1 + 360) % 360 }) < residual({ ...cal, rollDeg: (m2 + 360) % 360 })) hi = m2
      else lo = m1
    }
    best = { ...cal, rollDeg: (((lo + hi) / 2) % 360 + 360) % 360 }
    return residual(best) <= 2 ? best : null
  }

  const fixedPoint = fixedPointTiltedAim(poleXPx, poleYPx, cal, view, pole)
  if (residual(fixedPoint) <= 2) return fixedPoint
  const newton = newtonTiltedAim(poleXPx, poleYPx, cal, view, pole)
  return residual(newton) <= 2 ? newton : null
}
