import type { CropRect, LensCalibration } from '../api/types'

export type EditorHandle = 'center' | 'radius' | 'rotation'
export type CropHandle = 'tl' | 'br'

const MIN_CROP_PX = 100

const DEG = Math.PI / 180

/** Handle anchor points, same convention as altAzToImage: north = az 0. */
export function handlePositions(cal: LensCalibration) {
  const rot = cal.rotationDeg * DEG
  const sx = cal.flip ? -1 : 1
  const at = (angleRad: number) => ({
    x: cal.cx + sx * cal.radiusPx * Math.sin(angleRad),
    y: cal.cy - cal.radiusPx * Math.cos(angleRad),
  })
  return {
    center: { x: cal.cx, y: cal.cy },
    rotation: at(rot), // north marker on the horizon circle
    radius: at(rot + Math.PI / 2), // east marker
  }
}

export function hitTest(
  x: number, y: number, cal: LensCalibration, tolPx = 24,
): EditorHandle | null {
  const hp = handlePositions(cal)
  for (const h of ['rotation', 'radius', 'center'] as const) {
    if (Math.hypot(x - hp[h].x, y - hp[h].y) <= tolPx) return h
  }
  return null
}

export function applyDrag(
  handle: EditorHandle, x: number, y: number, cal: LensCalibration,
): LensCalibration {
  if (handle === 'center') return { ...cal, cx: x, cy: y }
  if (handle === 'radius') {
    return { ...cal, radiusPx: Math.max(20, Math.hypot(x - cal.cx, y - cal.cy)) }
  }
  const sx = cal.flip ? -1 : 1
  const ang = Math.atan2(sx * (x - cal.cx), cal.cy - y) / DEG
  return { ...cal, rotationDeg: (ang + 360) % 360 }
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
