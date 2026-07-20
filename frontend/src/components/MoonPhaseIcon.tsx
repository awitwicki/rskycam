import { useEffect, useRef } from 'react'

/** Moon phase disc: lit fraction `pct` (0–100); waxing = right side lit. */
export default function MoonPhaseIcon({ pct, waxing, size = 36 }: {
  pct: number
  waxing: boolean
  size?: number
}) {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')!
    const c = size / 2
    const r = size / 2 - 1
    const f = Math.min(1, Math.max(0, pct / 100))

    ctx.clearRect(0, 0, size, size)
    ctx.fillStyle = '#1a2334'
    ctx.beginPath()
    ctx.arc(c, c, r, 0, Math.PI * 2)
    ctx.fill()

    // lit semicircle (right when waxing, left when waning)
    ctx.fillStyle = '#e2e8f4'
    ctx.beginPath()
    ctx.arc(c, c, r, -Math.PI / 2, Math.PI / 2, !waxing)
    ctx.fill()

    // terminator ellipse completes or eats into the lit half
    ctx.fillStyle = f >= 0.5 ? '#e2e8f4' : '#1a2334'
    ctx.beginPath()
    ctx.ellipse(c, c, Math.abs(2 * f - 1) * r, r, 0, 0, Math.PI * 2)
    ctx.fill()

    ctx.strokeStyle = 'rgba(133,147,176,0.6)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.arc(c, c, r, 0, Math.PI * 2)
    ctx.stroke()
  }, [pct, waxing, size])

  return (
    <canvas ref={ref} role="img" style={{ width: size, height: size }}
      aria-label={`Moon ${Math.round(pct)}% illuminated, ${waxing ? 'waxing' : 'waning'}`} />
  )
}
