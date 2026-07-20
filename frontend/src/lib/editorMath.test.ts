import { describe, expect, it } from 'vitest'
import {
  applyCropDrag, applyDrag, cropHitTest, handlePositions, hitTest, textFieldHitTest,
} from './editorMath'

const cal = { cx: 480, cy: 480, radiusPx: 440, rotationDeg: 0, flip: false }

describe('handlePositions', () => {
  it('rotation handle sits at north, radius handle at east (rotation 0)', () => {
    const hp = handlePositions(cal)
    expect(hp.center).toEqual({ x: 480, y: 480 })
    expect(hp.rotation.x).toBeCloseTo(480)
    expect(hp.rotation.y).toBeCloseTo(40)
    expect(hp.radius.x).toBeCloseTo(920)
    expect(hp.radius.y).toBeCloseTo(480)
  })

  it('flip mirrors the radius handle to the west side', () => {
    const hp = handlePositions({ ...cal, flip: true })
    expect(hp.radius.x).toBeCloseTo(40)
  })
})

describe('hitTest', () => {
  it('finds each handle within tolerance, null elsewhere', () => {
    expect(hitTest(482, 478, cal)).toBe('center')
    expect(hitTest(480, 45, cal)).toBe('rotation')
    expect(hitTest(915, 480, cal)).toBe('radius')
    expect(hitTest(200, 200, cal)).toBeNull()
  })
})

describe('applyDrag', () => {
  it('center drag moves cx/cy', () => {
    const next = applyDrag('center', 400, 500, cal)
    expect(next.cx).toBe(400)
    expect(next.cy).toBe(500)
  })

  it('radius drag sets radius to pointer distance from center', () => {
    const next = applyDrag('radius', 480 + 300, 480, cal)
    expect(next.radiusPx).toBeCloseTo(300)
  })

  it('rotation drag: pointer due east sets rotation to 90°', () => {
    const next = applyDrag('rotation', 900, 480, cal)
    expect(next.rotationDeg).toBeCloseTo(90)
  })

  it('rotation drag respects flip', () => {
    const next = applyDrag('rotation', 900, 480, { ...cal, flip: true })
    expect(next.rotationDeg).toBeCloseTo(270)
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
