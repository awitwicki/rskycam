import type {
  CropRect, LensCalibration, LocationSettings, OverlayGeometry, OverlayLabel,
  OverlayLayers, OverlayPolyline,
} from '../api/types'
import { altAzToImage, lstDeg, raDecToAltAz, thetaMaxDeg, type LensView } from './astro'
import constellationsData from './constellations.json'

interface ConstellationDef {
  id: string
  name: string
  labelRaDeg: number
  labelDecDeg: number
  lines: [number, number][][]
}

const CONSTELLATIONS = constellationsData.constellations as ConstellationDef[]

/** Shift sensor-space geometry into cropped-image coordinates. */
export function cropGeometry(g: OverlayGeometry, crop: CropRect): OverlayGeometry {
  return {
    imageWidth: crop.width,
    imageHeight: crop.height,
    polylines: g.polylines.map((pl) => ({
      ...pl,
      points: pl.points.map(([x, y]) => [x - crop.x, y - crop.y] as [number, number]),
    })),
    labels: g.labels.map((l) => ({ ...l, x: l.x - crop.x, y: l.y - crop.y })),
  }
}

/** Manual fisheye mask circle in sensor-frame pixels. */
export interface MaskCircle {
  centerXPx: number
  centerYPx: number
  radiusPx: number
}

export interface BuildOverlayOptions {
  time: Date
  location: LocationSettings
  calibration: LensCalibration
  layers: OverlayLayers
  gridOpacity?: number // stamped onto altAz/raDec polylines
  constellationsOpacity?: number // stamped onto constellation polylines
  imageWidth: number
  imageHeight: number
  /** Native sensor width for plate scale; defaults to imageWidth (binning 1). */
  nativeWidth?: number
  /** When set, grid lines are culled outside this circle: the mask covers
   *  the grid, never the other way around. Labels are left alone. */
  mask?: MaskCircle
}

const MIN_ALT_RADEC = 0

interface VisSample { altDeg: number; thetaDeg: number; x: number; y: number }

/** Split a sampled line into segments inside the usable field of view:
 *  θ ≤ thetaMax, (when minAlt is set) above the horizon, and (when a mask
 *  circle is set) inside the mask. */
function visibleSegments(
  samples: VisSample[], minAlt: number | null, thetaMax: number, mask?: MaskCircle,
): [number, number][][] {
  const segs: [number, number][][] = []
  let cur: [number, number][] = []
  const inMask = (s: VisSample) => mask === undefined
    || (s.x - mask.centerXPx) ** 2 + (s.y - mask.centerYPx) ** 2 <= mask.radiusPx ** 2
  for (const s of samples) {
    if ((minAlt === null || s.altDeg >= minAlt) && s.thetaDeg <= thetaMax && inMask(s)) {
      cur.push([s.x, s.y])
    } else {
      if (cur.length > 1) segs.push(cur)
      cur = []
    }
  }
  if (cur.length > 1) segs.push(cur)
  return segs
}

export function buildOverlayGeometry(o: BuildOverlayOptions): OverlayGeometry {
  const { calibration: cal, layers } = o
  const view: LensView = {
    frameWidth: o.imageWidth,
    frameHeight: o.imageHeight,
    nativeWidth: o.nativeWidth ?? o.imageWidth,
  }
  const thetaMax = thetaMaxDeg(cal.lensType)
  const polylines: OverlayPolyline[] = []
  const labels: OverlayLabel[] = []

  const opacity = o.gridOpacity
  const lst = lstDeg(o.time, o.location.longitudeDeg)
  const lat = o.location.latitudeDeg
  const projectRaDec = (raDeg: number, decDeg: number): VisSample => {
    const { altDeg, azDeg } = raDecToAltAz(raDeg, decDeg, lat, lst)
    const { x, y, thetaDeg } = altAzToImage(altDeg, azDeg, cal, view)
    return { altDeg, thetaDeg, x, y }
  }

  if (layers.altAzGrid) {
    for (const alt of [0, 30, 60]) {
      const samples: VisSample[] = []
      for (let az = 0; az <= 360; az += 5) {
        const p = altAzToImage(alt, az, cal, view)
        samples.push({ altDeg: alt, thetaDeg: p.thetaDeg, x: p.x, y: p.y })
      }
      for (const points of visibleSegments(samples, null, thetaMax, o.mask)) polylines.push({ layer: 'altAz', points, opacity })
    }
    for (let az = 0; az < 360; az += 45) {
      const samples: VisSample[] = []
      for (let alt = 0; alt <= 80; alt += 5) {
        const p = altAzToImage(alt, az, cal, view)
        samples.push({ altDeg: alt, thetaDeg: p.thetaDeg, x: p.x, y: p.y })
      }
      for (const points of visibleSegments(samples, null, thetaMax, o.mask)) polylines.push({ layer: 'altAz', points, opacity })
    }
  }

  if (layers.cardinal) {
    const cardinals: [string, number][] = [['N', 0], ['E', 90], ['S', 180], ['W', 270]]
    for (const [text, az] of cardinals) {
      const p = altAzToImage(-8, az, cal, view) // a bit outside the horizon circle
      if (p.thetaDeg > thetaMax) continue
      labels.push({ layer: 'cardinal', text, x: p.x, y: p.y, fontSize: 28 })
    }
  }

  if (layers.raDecGrid) {
    // ±80 keeps a small circle around each celestial pole so the grid
    // doesn't leave a hole there.
    for (const dec of [-80, -60, -30, 0, 30, 60, 80]) {
      const samples: VisSample[] = []
      for (let ra = 0; ra <= 360; ra += 3) samples.push(projectRaDec(ra, dec))
      for (const points of visibleSegments(samples, MIN_ALT_RADEC, thetaMax, o.mask)) polylines.push({ layer: 'raDec', points, opacity })
    }
    // Meridians run to dec ±90 so they converge exactly at the poles.
    for (let ra = 0; ra < 360; ra += 30) {
      const samples: VisSample[] = []
      for (let dec = -90; dec <= 90; dec += 3) samples.push(projectRaDec(ra, dec))
      for (const points of visibleSegments(samples, MIN_ALT_RADEC, thetaMax, o.mask)) polylines.push({ layer: 'raDec', points, opacity })
    }
  }

  if (layers.constellations) {
    const constellationsOpacity = o.constellationsOpacity
    for (const c of CONSTELLATIONS) {
      for (const line of c.lines) {
        const samples = line.map(([raDeg, decDeg]) => projectRaDec(raDeg, decDeg))
        for (const points of visibleSegments(samples, MIN_ALT_RADEC, thetaMax, o.mask)) {
          polylines.push({ layer: 'constellations', points, opacity: constellationsOpacity })
        }
      }
      const label = projectRaDec(c.labelRaDeg, c.labelDecDeg)
      if (label.altDeg >= MIN_ALT_RADEC && label.thetaDeg <= thetaMax) {
        labels.push({ layer: 'constellationLabels', text: c.name, x: label.x, y: label.y, fontSize: 13 })
      }
    }
  }

  return { imageWidth: o.imageWidth, imageHeight: o.imageHeight, polylines, labels }
}
