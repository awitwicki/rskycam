import { useEffect, useRef } from 'react'
import type { LightgraphData } from '../api/types'

const BAR_H = 24

// Twilight bands by sun altitude (canvas literals allowed).
function bandColor(altDeg: number): string {
  if (altDeg > 0) return '#3d5a8a' // day
  if (altDeg > -6) return '#2b4066' // civil twilight
  if (altDeg > -12) return '#1d2c49' // nautical
  if (altDeg > -18) return '#141d33' // astronomical
  return '#080b13' // night
}

function hourLabel(ms: number): string {
  return `${String(new Date(ms).getHours()).padStart(2, '0')}:00`
}

/** 24-hour day/twilight/night bar with a "now" marker. */
export default function Lightgraph({ data }: { data: LightgraphData }) {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    const n = data.sunAltDeg.length
    canvas.width = n
    canvas.height = BAR_H
    const ctx = canvas.getContext('2d')!
    data.sunAltDeg.forEach((alt, i) => {
      ctx.fillStyle = bandColor(alt)
      ctx.fillRect(i, 0, 1, BAR_H)
    })
    const start = new Date(data.startIso).getTime()
    const idx = (Date.now() - start) / 60_000 / data.stepMinutes
    if (idx >= 0 && idx < n) {
      ctx.fillStyle = '#4cc9f0'
      ctx.fillRect(Math.round(idx) - 1, 0, 2, BAR_H)
    }
  }, [data])

  const start = new Date(data.startIso).getTime()
  const totalMs = data.sunAltDeg.length * data.stepMinutes * 60_000
  return (
    <div>
      <canvas ref={ref} role="img" aria-label="24-hour daylight graph"
        className="h-6 w-full rounded border border-line" />
      <div className="mt-0.5 flex justify-between font-mono text-[10px] text-fgdim">
        <span>{hourLabel(start)}</span>
        <span>{hourLabel(start + totalMs / 2)}</span>
        <span>{hourLabel(start + totalMs)}</span>
      </div>
    </div>
  )
}
