import { useEffect, useRef } from 'react'
import type { OverlayGeometry } from '../api/types'

const LAYER_STYLE: Record<string, string> = {
  // Grid lines are full-strength here; transparency comes from the
  // polyline's own opacity (settings.overlay.gridOpacity).
  altAz: 'rgb(76,201,240)',
  raDec: 'rgb(240,164,76)',
  cardinal: 'rgba(226,232,244,0.9)',
  text: 'rgba(226,232,244,0.95)',
}

export function drawOverlay(ctx: CanvasRenderingContext2D, g: OverlayGeometry) {
  ctx.clearRect(0, 0, g.imageWidth, g.imageHeight)
  ctx.lineWidth = 1.2
  for (const pl of g.polylines) {
    ctx.globalAlpha = pl.opacity ?? 1
    ctx.strokeStyle = LAYER_STYLE[pl.layer] ?? 'rgba(255,255,255,0.4)'
    ctx.beginPath()
    pl.points.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)))
    ctx.stroke()
  }
  ctx.globalAlpha = 1
  ctx.textBaseline = 'middle'
  for (const l of g.labels) {
    ctx.textAlign = l.align ?? 'center'
    ctx.fillStyle = LAYER_STYLE[l.layer] ?? '#fff'
    ctx.font = `${l.fontSize}px ui-monospace, monospace`
    ctx.fillText(l.text, l.x, l.y)
  }
}

export default function OverlayCanvas({ geometry, className }: {
  geometry: OverlayGeometry; className?: string
}) {
  const ref = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    canvas.width = geometry.imageWidth
    canvas.height = geometry.imageHeight
    drawOverlay(canvas.getContext('2d')!, geometry)
  }, [geometry])
  return <canvas ref={ref} className={className} aria-hidden="true" />
}
