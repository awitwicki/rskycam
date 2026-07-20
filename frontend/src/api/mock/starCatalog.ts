export interface Star {
  raDeg: number
  decDeg: number
  mag: number
}

/** Small deterministic PRNG. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Uniformly distributed fake stars on the celestial sphere. */
export function makeStarCatalog(count: number, seed = 42): Star[] {
  const rnd = mulberry32(seed)
  return Array.from({ length: count }, () => ({
    raDeg: rnd() * 360,
    decDeg: (Math.asin(2 * rnd() - 1) * 180) / Math.PI,
    mag: 1 + rnd() * 5,
  }))
}
