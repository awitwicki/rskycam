import type { CropRect, LensCalibration, LocationSettings, MaskMode } from '../types'
import { altAzToImage, lstDeg, raDecToAltAz } from '../../lib/astro'
import { makeStarCatalog } from './starCatalog'

const CATALOG = makeStarCatalog(600)

/** Mock sensor dimensions — matches the ASI120MM Mini (1280×960). */
export const SENSOR_W = 1280
export const SENSOR_H = 960

export interface RenderSkyOptions {
  time: Date
  location: LocationSettings
  calibration: LensCalibration // sensor-space
  maskMode: MaskMode
  crop: CropRect | null // sensor-space; null = full frame
  scale?: number // output px per sensor px (default 1)
  /** Real sample photo used as the frame when loaded; synthetic sky otherwise. */
  photo?: HTMLImageElement | null
}

/** Renders the latest mock frame as a JPEG data URL: real sample photo (or a
 *  synthetic starfield whose stars use the same projection as the overlay
 *  grid), then the circle mask, then the crop — same order as the real
 *  pipeline. */
export function renderSky(o: RenderSkyOptions): string {
  const sensor = document.createElement('canvas')
  sensor.width = SENSOR_W
  sensor.height = SENSOR_H
  const ctx = sensor.getContext('2d')!
  const cal = o.calibration
  const circle = o.maskMode === 'circle'

  ctx.fillStyle = '#05070d'
  ctx.fillRect(0, 0, SENSOR_W, SENSOR_H)

  ctx.save()
  if (circle) {
    ctx.beginPath()
    ctx.arc(cal.cx, cal.cy, cal.radiusPx, 0, Math.PI * 2)
    ctx.clip()
  }

  const photoReady = o.photo && o.photo.complete && o.photo.naturalWidth > 0
  if (photoReady) {
    ctx.drawImage(o.photo!, 0, 0, SENSOR_W, SENSOR_H)
  } else {
    drawSyntheticSky(ctx, o, circle)
  }
  ctx.restore()

  const crop = o.crop ?? { x: 0, y: 0, width: SENSOR_W, height: SENSOR_H }
  const k = o.scale ?? 1
  const out = document.createElement('canvas')
  out.width = Math.max(1, Math.round(crop.width * k))
  out.height = Math.max(1, Math.round(crop.height * k))
  out.getContext('2d')!
    .drawImage(sensor, crop.x, crop.y, crop.width, crop.height, 0, 0, out.width, out.height)
  return out.toDataURL('image/jpeg', 0.85)
}

function drawSyntheticSky(
  ctx: CanvasRenderingContext2D, o: RenderSkyOptions, circle: boolean,
) {
  const cal = o.calibration
  const g = ctx.createRadialGradient(
    cal.cx, cal.cy, 0, cal.cx, cal.cy, circle ? cal.radiusPx : cal.radiusPx * 1.6,
  )
  g.addColorStop(0, '#0b1226')
  g.addColorStop(0.8, '#0a0f20')
  g.addColorStop(1, '#131a2e')
  ctx.fillStyle = g
  ctx.fillRect(0, 0, SENSOR_W, SENSOR_H)

  const lst = lstDeg(o.time, o.location.longitudeDeg)
  // The full frame extends past the horizon circle on a real sensor, so let
  // below-horizon stars fill the corners in full-frame mode.
  const minAlt = circle ? 0 : -55
  for (const s of CATALOG) {
    const { altDeg, azDeg } = raDecToAltAz(s.raDeg, s.decDeg, o.location.latitudeDeg, lst)
    if (altDeg < minAlt) continue
    const { x, y } = altAzToImage(altDeg, azDeg, cal)
    const b = Math.max(0, (6.5 - s.mag) / 5.5)
    ctx.beginPath()
    ctx.arc(x, y, 0.6 + b * 1.6, 0, Math.PI * 2)
    ctx.fillStyle = `rgba(226,232,244,${0.25 + 0.75 * b})`
    ctx.fill()
  }
}
