import { mulberry32 } from './starCatalog'

/** Fake keogram: bright twilight blue at both edges, dark night in the
 *  middle, sprinkled star pixels. */
export function renderKeogram(width = 720, height = 220, seed = 7): string {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')!
  const rnd = mulberry32(seed)
  for (let x = 0; x < width; x++) {
    const d = Math.sin((x / width) * Math.PI) // 0 at edges → 1 mid-night
    const r = Math.round(42 - 34 * d)
    const g = Math.round(58 - 47 * d)
    const b = Math.round(106 - 84 * d)
    ctx.fillStyle = `rgb(${r},${g},${b})`
    ctx.fillRect(x, 0, 1, height)
    if (rnd() < 0.8) {
      ctx.fillStyle = 'rgba(230,236,250,0.7)'
      ctx.fillRect(x, rnd() * height, 1, 1)
    }
  }
  return canvas.toDataURL('image/jpeg', 0.85)
}

/** Fake star trails: concentric arcs around the celestial pole. */
export function renderStartrails(size = 720, seed = 11): string {
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size
  const ctx = canvas.getContext('2d')!
  ctx.fillStyle = '#05070d'
  ctx.fillRect(0, 0, size, size)
  ctx.save()
  ctx.beginPath()
  ctx.arc(size / 2, size / 2, size * 0.48, 0, Math.PI * 2)
  ctx.clip()
  const rnd = mulberry32(seed)
  const px = size / 2
  const py = size * 0.38
  for (let i = 0; i < 240; i++) {
    const r = rnd() * size * 0.55
    const a0 = rnd() * Math.PI * 2
    ctx.beginPath()
    ctx.arc(px, py, r, a0, a0 + 0.26)
    ctx.strokeStyle = `rgba(226,232,244,${0.15 + rnd() * 0.5})`
    ctx.lineWidth = rnd() < 0.15 ? 1.6 : 0.8
    ctx.stroke()
  }
  ctx.restore()
  return canvas.toDataURL('image/jpeg', 0.85)
}
