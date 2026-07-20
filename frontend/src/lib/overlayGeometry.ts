import type {
  CropRect, LensCalibration, LocationSettings, OverlayGeometry, OverlayLabel,
  OverlayLayers, OverlayPolyline,
} from '../api/types'
import { altAzToImage, lstDeg, raDecToAltAz } from './astro'

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

export interface BuildOverlayOptions {
  time: Date
  location: LocationSettings
  calibration: LensCalibration
  layers: OverlayLayers
  gridOpacity?: number // stamped onto altAz/raDec polylines
  imageWidth: number
  imageHeight: number
}

const MIN_ALT_RADEC = 2

/** Split a sampled line into segments that stay above the horizon. */
function segmentsAboveHorizon(
  samples: { altDeg: number; x: number; y: number }[],
): [number, number][][] {
  const segs: [number, number][][] = []
  let cur: [number, number][] = []
  for (const s of samples) {
    if (s.altDeg >= MIN_ALT_RADEC) {
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
  const polylines: OverlayPolyline[] = []
  const labels: OverlayLabel[] = []

  const opacity = o.gridOpacity

  if (layers.altAzGrid) {
    for (const alt of [0, 30, 60]) {
      const points: [number, number][] = []
      for (let az = 0; az <= 360; az += 5) {
        const p = altAzToImage(alt, az, cal)
        points.push([p.x, p.y])
      }
      polylines.push({ layer: 'altAz', points, opacity })
    }
    for (let az = 0; az < 360; az += 45) {
      const points: [number, number][] = []
      for (let alt = 0; alt <= 80; alt += 5) {
        const p = altAzToImage(alt, az, cal)
        points.push([p.x, p.y])
      }
      polylines.push({ layer: 'altAz', points, opacity })
    }
  }

  if (layers.cardinal) {
    const cardinals: [string, number][] = [['N', 0], ['E', 90], ['S', 180], ['W', 270]]
    for (const [text, az] of cardinals) {
      const p = altAzToImage(-8, az, cal) // a bit outside the horizon circle
      labels.push({ layer: 'cardinal', text, x: p.x, y: p.y, fontSize: 28 })
    }
  }

  if (layers.raDecGrid) {
    const lst = lstDeg(o.time, o.location.longitudeDeg)
    const lat = o.location.latitudeDeg
    const sample = (raDeg: number, decDeg: number) => {
      const { altDeg, azDeg } = raDecToAltAz(raDeg, decDeg, lat, lst)
      const { x, y } = altAzToImage(altDeg, azDeg, cal)
      return { altDeg, x, y }
    }
    // ±80 keeps a small circle around each celestial pole so the grid
    // doesn't leave a hole there.
    for (const dec of [-80, -60, -30, 0, 30, 60, 80]) {
      const samples = []
      for (let ra = 0; ra <= 360; ra += 3) samples.push(sample(ra, dec))
      for (const points of segmentsAboveHorizon(samples)) polylines.push({ layer: 'raDec', points, opacity })
    }
    // Meridians run to dec ±90 so they converge exactly at the poles.
    for (let ra = 0; ra < 360; ra += 30) {
      const samples = []
      for (let dec = -90; dec <= 90; dec += 3) samples.push(sample(ra, dec))
      for (const points of segmentsAboveHorizon(samples)) polylines.push({ layer: 'raDec', points, opacity })
    }
  }

  return { imageWidth: o.imageWidth, imageHeight: o.imageHeight, polylines, labels }
}
