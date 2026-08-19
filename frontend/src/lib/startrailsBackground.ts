import type { CSSProperties } from 'react'
import type { CropRect } from '../api/types'

export interface Dims {
  w: number
  h: number
}

/** How a startrails image maps into the editor's sensor-space stage. */
export type StartrailsFit =
  | { kind: 'exact' } // same size as the raw sensor frame
  | { kind: 'cropOffset'; rect: CropRect } // matches the configured crop
  | { kind: 'mismatch' } // different crop or camera — alignment unreliable

/** The backend truncates fractional crop dims (`c.width as u32`), so a
 *  ±1 px slack keeps float settings and on-disk pixels agreeing. */
function near(a: number, b: number): boolean {
  return Math.abs(a - b) <= 1
}

export function classifyStartrailsFit(
  dims: Dims,
  sensor: Dims,
  crop: CropRect | null,
): StartrailsFit {
  if (dims.w === sensor.w && dims.h === sensor.h) return { kind: 'exact' }
  if (crop && near(dims.w, crop.width) && near(dims.h, crop.height)) {
    return { kind: 'cropOffset', rect: crop }
  }
  return { kind: 'mismatch' }
}

/** Absolute placement of the startrails <img> inside the sensor-sized stage,
 *  in percent so it survives responsive scaling of the stage. */
export function startrailsImgStyle(fit: StartrailsFit, sensor: Dims): CSSProperties {
  if (fit.kind === 'cropOffset') {
    return {
      position: 'absolute',
      left: `${(fit.rect.x / sensor.w) * 100}%`,
      top: `${(fit.rect.y / sensor.h) * 100}%`,
      width: `${(fit.rect.width / sensor.w) * 100}%`,
      height: `${(fit.rect.height / sensor.h) * 100}%`,
    }
  }
  return {
    position: 'absolute', left: '0%', top: '0%', width: '100%', height: '100%',
    ...(fit.kind === 'mismatch' ? { objectFit: 'contain' as const } : {}),
  }
}

/** Thumbnail variant of an artifact URL; data URLs (mock API) pass through. */
export function thumbUrl(url: string): string {
  return url.startsWith('data:') ? url : `${url}?thumb=1`
}
