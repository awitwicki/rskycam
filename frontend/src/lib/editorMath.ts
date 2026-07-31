import type { CropRect, ImageSettings, LensCalibration } from '../api/types'
import {
  camBasis, focalLengthPx, opticalCenter, thetaMaxDeg, type LensView, type Vec3,
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
