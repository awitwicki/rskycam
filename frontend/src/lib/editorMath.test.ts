import { describe, expect, it } from 'vitest'
import type { ImageSettings } from '../api/types'
import { altAzToImage } from './astro'
import {
  applyCenterPan, applyCropDrag, applyMaskDrag, applyRollDrag, applySkyPan,
  applyWheelZoom, calibrationHitTest, cropHitTest, imageToAltAz,
  maskHandlePositions, maskHitTest, rollHandlePosition, textFieldHitTest,
} from './editorMath'

const cal = {
  lensType: 'fisheye' as const,
  focalLengthMm: 0.88 / Math.PI,
  pixelSizeUm: 1,
  pointingAzDeg: 0,
  pointingAltDeg: 90,
  rollDeg: 0,
  flip: false,
  centerOffsetXPx: 0,
  centerOffsetYPx: 0,
}
const view = { frameWidth: 960, frameHeight: 960, nativeWidth: 960 }

describe('imageToAltAz', () => {
  it('is the inverse of altAzToImage', () => {
    for (const [alt, az] of [[50, 120], [10, 300], [80, 45]] as const) {
      const p = altAzToImage(alt, az, cal, view)
      const r = imageToAltAz(p.x, p.y, cal, view)
      expect(r.altDeg).toBeCloseTo(alt, 6)
      expect(r.azDeg).toBeCloseTo(az, 6)
    }
  })

  it('round-trips for tilted and rectilinear calibrations', () => {
    const tilted = { ...cal, pointingAzDeg: 180, pointingAltDeg: 40, rollDeg: 30 }
    const p = altAzToImage(55, 200, tilted, view)
    const r = imageToAltAz(p.x, p.y, tilted, view)
    expect(r.altDeg).toBeCloseTo(55, 6)
    expect(r.azDeg).toBeCloseTo(200, 6)
    const rect = { ...cal, lensType: 'rectilinear' as const }
    const q = altAzToImage(60, 10, rect, view)
    const s = imageToAltAz(q.x, q.y, rect, view)
    expect(s.altDeg).toBeCloseTo(60, 6)
    expect(s.azDeg).toBeCloseTo(10, 6)
  })

  it('maps the legacy horizon-north pixel back to alt 0 az 0', () => {
    const r = imageToAltAz(480, 40, cal, view)
    expect(r.altDeg).toBeCloseTo(0, 6)
    expect(r.azDeg).toBeCloseTo(0, 6)
  })
})

describe('applySkyPan', () => {
  it('same point → calibration unchanged', () => {
    const p = altAzToImage(45, 30, cal, view)
    const grab = imageToAltAz(p.x, p.y, cal, view)
    const next = applySkyPan(grab, p.x, p.y, cal, view)
    expect(next.pointingAzDeg).toBeCloseTo(0, 6)
    expect(next.pointingAltDeg).toBeCloseTo(90, 6)
  })

  it('dragging a grabbed point onto another sky position shifts pointing by the delta', () => {
    // Grab the sky at (alt 45, az 0), drop the cursor on the pixel where
    // (alt 45, az 10) currently sits → pointing azimuth moves −10° (wraps).
    const grab = { altDeg: 45, azDeg: 0 }
    const target = altAzToImage(45, 10, cal, view)
    const next = applySkyPan(grab, target.x, target.y, cal, view)
    expect(next.pointingAzDeg).toBeCloseTo(350, 5)
    expect(next.pointingAltDeg).toBeCloseTo(90, 5)
  })

  it('clamps pointing altitude to ±90', () => {
    const c = { ...cal, pointingAltDeg: 89 }
    const grab = { altDeg: 40, azDeg: 180 }
    const target = altAzToImage(30, 180, c, view) // drag the sky 10° "down"
    const next = applySkyPan(grab, target.x, target.y, c, view)
    expect(next.pointingAltDeg).toBeLessThanOrEqual(90)
  })
})

describe('roll handle', () => {
  it('sits at 0.35·min(W,H) above the optical center at roll 0', () => {
    const h = rollHandlePosition(cal, view)
    expect(h.x).toBeCloseTo(480)
    expect(h.y).toBeCloseTo(480 - 0.35 * 960)
  })

  it('hit-tests the handle and misses elsewhere', () => {
    expect(calibrationHitTest(480, 480 - 336, cal, view)).toBe('roll')
    expect(calibrationHitTest(480, 480, cal, view)).toBeNull()
  })

  it('drag due east sets roll 90; flip mirrors to 270', () => {
    expect(applyRollDrag(900, 480, cal, view).rollDeg).toBeCloseTo(90)
    expect(applyRollDrag(900, 480, { ...cal, flip: true }, view).rollDeg).toBeCloseTo(270)
  })
})

describe('applyWheelZoom', () => {
  it('scroll down shrinks focal length, scroll up grows it, both clamped', () => {
    expect(applyWheelZoom(cal, 100).focalLengthMm).toBeLessThan(cal.focalLengthMm)
    expect(applyWheelZoom(cal, -100).focalLengthMm).toBeGreaterThan(cal.focalLengthMm)
    expect(applyWheelZoom({ ...cal, focalLengthMm: 0.1 }, 10_000).focalLengthMm).toBe(0.1)
    expect(applyWheelZoom({ ...cal, focalLengthMm: 100 }, -10_000).focalLengthMm).toBe(100)
  })
})

describe('applyCenterPan', () => {
  it('moves the optical-center offsets by the pointer delta from the grab point', () => {
    const grab = { offsetX: 10, offsetY: -20, x: 500, y: 400 }
    const next = applyCenterPan(grab, 530, 380, cal)
    expect(next.centerOffsetXPx).toBe(40)
    expect(next.centerOffsetYPx).toBe(-40)
  })

  it('same point → offsets unchanged', () => {
    const grab = { offsetX: 7, offsetY: 9, x: 100, y: 100 }
    const next = applyCenterPan(grab, 100, 100, cal)
    expect(next.centerOffsetXPx).toBe(7)
    expect(next.centerOffsetYPx).toBe(9)
  })

  it('clamps offsets to the sanitize bounds (±5000)', () => {
    const grab = { offsetX: 4990, offsetY: -4990, x: 0, y: 0 }
    const next = applyCenterPan(grab, 100, -100, cal)
    expect(next.centerOffsetXPx).toBe(5000)
    expect(next.centerOffsetYPx).toBe(-5000)
  })
})

describe('mask circle handles', () => {
  const img: ImageSettings = {
    maskMode: 'circle', maskCenterXPx: 640, maskCenterYPx: 480, maskRadiusPx: 620, crop: null,
  }

  it('positions: center dot and a radius dot on the east edge', () => {
    const hp = maskHandlePositions(img)
    expect(hp.maskCenter).toEqual({ x: 640, y: 480 })
    expect(hp.maskRadius).toEqual({ x: 1260, y: 480 })
  })

  it('hit-tests both handles, misses elsewhere', () => {
    expect(maskHitTest(642, 478, img)).toBe('maskCenter')
    expect(maskHitTest(1255, 482, img)).toBe('maskRadius')
    expect(maskHitTest(900, 100, img)).toBeNull()
  })

  it('center drag moves the circle', () => {
    const next = applyMaskDrag('maskCenter', 500, 400, img)
    expect(next.maskCenterXPx).toBe(500)
    expect(next.maskCenterYPx).toBe(400)
    expect(next.maskRadiusPx).toBe(620)
  })

  it('radius drag sets radius to pointer distance, min 20', () => {
    const next = applyMaskDrag('maskRadius', 640 + 300, 480, img)
    expect(next.maskRadiusPx).toBeCloseTo(300)
    expect(applyMaskDrag('maskRadius', 641, 480, img).maskRadiusPx).toBe(20)
  })
})

describe('crop handles', () => {
  const rect = { x: 100, y: 80, width: 600, height: 500 }

  it('hit-tests both corners and misses the interior', () => {
    expect(cropHitTest(105, 82, rect)).toBe('tl')
    expect(cropHitTest(695, 578, rect)).toBe('br')
    expect(cropHitTest(400, 300, rect)).toBeNull()
  })

  it('tl drag moves the origin and keeps the opposite corner fixed', () => {
    expect(applyCropDrag('tl', 150, 120, rect, 1280, 960))
      .toEqual({ x: 150, y: 120, width: 550, height: 460 })
  })

  it('br drag resizes from the fixed origin', () => {
    expect(applyCropDrag('br', 800, 700, rect, 1280, 960))
      .toEqual({ x: 100, y: 80, width: 700, height: 620 })
  })

  it('clamps to sensor bounds and minimum size', () => {
    const tiny = applyCropDrag('br', 110, 90, rect, 1280, 960)
    expect(tiny.width).toBeGreaterThanOrEqual(100)
    expect(tiny.height).toBeGreaterThanOrEqual(100)
    const out = applyCropDrag('br', 5000, 5000, rect, 1280, 960)
    expect(out.width).toBe(1280 - rect.x)
    expect(out.height).toBe(960 - rect.y)
  })
})

describe('textFieldHitTest', () => {
  const boxes = [
    { id: 'a', x: 24, y: 40, fontSize: 24, width: 200 },
    { id: 'b', x: 24, y: 80, fontSize: 18, width: 120 },
  ]

  it('hits a field inside its padded box, first match wins', () => {
    expect(textFieldHitTest(100, 42, boxes)).toBe('a')
    expect(textFieldHitTest(30, 84, boxes)).toBe('b')
  })

  it('misses outside all boxes', () => {
    expect(textFieldHitTest(500, 42, boxes)).toBeNull()
    expect(textFieldHitTest(100, 200, boxes)).toBeNull()
  })
})
