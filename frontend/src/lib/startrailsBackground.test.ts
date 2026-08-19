import { describe, expect, it } from 'vitest'
import {
  classifyStartrailsFit, startrailsImgStyle, thumbUrl,
} from './startrailsBackground'

const sensor = { w: 1280, h: 960 }

describe('classifyStartrailsFit', () => {
  it('matches the sensor frame exactly when the night had no crop', () => {
    expect(classifyStartrailsFit({ w: 1280, h: 960 }, sensor, null))
      .toEqual({ kind: 'exact' })
  })

  it('prefers exact even when a crop is currently configured', () => {
    // Startrails from a night captured before the crop existed.
    const crop = { x: 100, y: 50, width: 800, height: 600 }
    expect(classifyStartrailsFit({ w: 1280, h: 960 }, sensor, crop))
      .toEqual({ kind: 'exact' })
  })

  it('maps crop-sized startrails to the crop position', () => {
    const crop = { x: 128, y: 96, width: 640, height: 480 }
    expect(classifyStartrailsFit({ w: 640, h: 480 }, sensor, crop))
      .toEqual({ kind: 'cropOffset', rect: crop })
  })

  it('tolerates the backend truncating fractional crop dims', () => {
    // apply_crop does `c.width as u32`, so 800.9 → 800 on disk.
    const crop = { x: 100.6, y: 50.2, width: 800.9, height: 600.4 }
    expect(classifyStartrailsFit({ w: 800, h: 600 }, sensor, crop))
      .toEqual({ kind: 'cropOffset', rect: crop })
  })

  it('reports mismatch for dims matching neither sensor nor crop', () => {
    expect(classifyStartrailsFit({ w: 720, h: 720 }, sensor, null))
      .toEqual({ kind: 'mismatch' })
    const crop = { x: 0, y: 0, width: 640, height: 480 }
    expect(classifyStartrailsFit({ w: 720, h: 720 }, sensor, crop))
      .toEqual({ kind: 'mismatch' })
  })
})

describe('startrailsImgStyle', () => {
  it('covers the whole stage for an exact fit', () => {
    expect(startrailsImgStyle({ kind: 'exact' }, sensor)).toEqual({
      position: 'absolute', left: '0%', top: '0%', width: '100%', height: '100%',
    })
  })

  it('places a cropped startrails at the crop position, in percent', () => {
    const fit = {
      kind: 'cropOffset' as const,
      rect: { x: 128, y: 96, width: 640, height: 480 },
    }
    expect(startrailsImgStyle(fit, sensor)).toEqual({
      position: 'absolute', left: '10%', top: '10%', width: '50%', height: '50%',
    })
  })

  it('letterboxes a mismatched image inside the stage', () => {
    expect(startrailsImgStyle({ kind: 'mismatch' }, sensor)).toEqual({
      position: 'absolute', left: '0%', top: '0%', width: '100%', height: '100%',
      objectFit: 'contain',
    })
  })
})

describe('thumbUrl', () => {
  it('appends the thumb query to server file URLs', () => {
    expect(thumbUrl('/api/files/2026-08-17/startrails.jpg'))
      .toBe('/api/files/2026-08-17/startrails.jpg?thumb=1')
  })

  it('leaves data URLs alone (mock API)', () => {
    const data = 'data:image/png;base64,abc'
    expect(thumbUrl(data)).toBe(data)
  })
})
