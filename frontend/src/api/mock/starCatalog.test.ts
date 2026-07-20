import { describe, it, expect } from 'vitest'
import { makeStarCatalog } from './starCatalog'

describe('makeStarCatalog', () => {
  it('is deterministic for the same seed', () => {
    expect(makeStarCatalog(50, 42)).toEqual(makeStarCatalog(50, 42))
  })

  it('differs for different seeds', () => {
    expect(makeStarCatalog(50, 1)).not.toEqual(makeStarCatalog(50, 2))
  })

  it('generates count stars within valid ranges', () => {
    const stars = makeStarCatalog(500, 7)
    expect(stars).toHaveLength(500)
    for (const s of stars) {
      expect(s.raDeg).toBeGreaterThanOrEqual(0)
      expect(s.raDeg).toBeLessThan(360)
      expect(Math.abs(s.decDeg)).toBeLessThanOrEqual(90)
      expect(s.mag).toBeGreaterThanOrEqual(1)
      expect(s.mag).toBeLessThanOrEqual(6)
    }
  })
})
