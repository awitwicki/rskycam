import { useEffect, useRef, useState, type PointerEvent } from 'react'
import { getApi } from '../api/client'
import type { FocusMeta } from '../api/types'

const MIN_ZOOM = 1
const MAX_ZOOM = 8

const EXPOSURE_LADDER_US = [100, 500, 1_000, 5_000, 20_000, 100_000, 500_000, 1_000_000, 2_000_000]
const DEFAULT_EXPOSURE_US = 1_000_000
const MAX_SAMPLES = 300

type Sample = { t: number; hfd: number }

function formatExposureUs(us: number): string {
  if (us < 1_000) return `${us} µs`
  if (us < 1_000_000) return `${us / 1_000} ms`
  const s = us / 1_000_000
  return `${s % 1 === 0 ? s : s.toFixed(1)} s`
}

/** "Nice" exposure steps, always including the connected camera's real
 * minimum even when it doesn't land on one of the round values — a
 * night-astrophotography floor would leave daytime testing permanently
 * overexposed. */
function exposureOptions(minUs: number): number[] {
  const above = EXPOSURE_LADDER_US.filter((us) => us >= minUs)
  return above[0] === minUs ? above : [minUs, ...above]
}

/** Evenly-spaced gain steps across the configured auto-exposure bounds. */
function gainOptions(min: number, max: number): number[] {
  if (max <= min) return [min]
  const steps = 6
  return Array.from({ length: steps },
    (_, i) => Math.round((min + ((max - min) * i) / (steps - 1)) * 10) / 10)
}

function nearestOption(value: number, options: number[]): number {
  return options.reduce((best, o) => (Math.abs(o - value) < Math.abs(best - value) ? o : best), options[0])
}

/** Full-frame preview with cursor-centered wheel zoom and drag-to-pan —
 * the server sends the raw frame uncropped, so the browser owns framing. */
function ZoomableImage({ src, alt }: { src: string; alt: string }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [scale, setScale] = useState(MIN_ZOOM)
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  const [naturalSize, setNaturalSize] = useState<{ w: number; h: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  const dragStart = useRef<{ x: number; y: number } | null>(null)
  // Read fresh inside the wheel listener below without re-subscribing it
  // on every zoom/pan (the effect only attaches once, so its closure
  // would otherwise see the initial scale/offset forever).
  const scaleRef = useRef(scale)
  scaleRef.current = scale
  const offsetRef = useRef(offset)
  offsetRef.current = offset

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    // A React onWheel prop can't reliably preventDefault (passive by
    // default), so this listens on the raw DOM node instead — same
    // pattern as OverlayEditorPage's calibration wheel-zoom.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const rect = container.getBoundingClientRect()
      const mouseX = e.clientX - rect.left
      const mouseY = e.clientY - rect.top
      const prevScale = scaleRef.current
      const prevOffset = offsetRef.current
      const factor = Math.exp(-e.deltaY * 0.001)
      const nextScale = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, prevScale * factor))
      // Keep the image point under the cursor fixed while it scales.
      const nextOffset = nextScale === MIN_ZOOM
        ? { x: 0, y: 0 }
        : {
            x: mouseX - ((mouseX - prevOffset.x) / prevScale) * nextScale,
            y: mouseY - ((mouseY - prevOffset.y) / prevScale) * nextScale,
          }
      setScale(nextScale)
      setOffset(nextOffset)
    }
    container.addEventListener('wheel', onWheel, { passive: false })
    return () => container.removeEventListener('wheel', onWheel)
  }, [])

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (scale <= MIN_ZOOM) return
    e.currentTarget.setPointerCapture(e.pointerId)
    dragStart.current = { x: e.clientX - offset.x, y: e.clientY - offset.y }
    setDragging(true)
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!dragStart.current) return
    setOffset({ x: e.clientX - dragStart.current.x, y: e.clientY - dragStart.current.y })
  }
  const endDrag = () => {
    dragStart.current = null
    setDragging(false)
  }

  return (
    <div ref={containerRef}
      className="relative overflow-hidden rounded-lg bg-black/20 select-none"
      style={{ aspectRatio: naturalSize ? `${naturalSize.w} / ${naturalSize.h}` : '4 / 3' }}
      onPointerDown={onPointerDown} onPointerMove={onPointerMove}
      onPointerUp={endDrag} onPointerLeave={endDrag}>
      <img src={src} alt={alt} draggable={false}
        onLoad={(e) => {
          const img = e.currentTarget
          setNaturalSize((prev) => (prev?.w === img.naturalWidth && prev?.h === img.naturalHeight
            ? prev
            : { w: img.naturalWidth, h: img.naturalHeight }))
        }}
        style={{
          width: '100%', display: 'block', transformOrigin: '0 0',
          transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
          cursor: scale > MIN_ZOOM ? (dragging ? 'grabbing' : 'grab') : 'default',
        }} />
      {scale > MIN_ZOOM && (
        <button onClick={() => { setScale(MIN_ZOOM); setOffset({ x: 0, y: 0 }) }}
          className="absolute right-2 top-2 rounded bg-black/60 px-2 py-1 text-xs text-white hover:bg-black/80">
          reset zoom
        </button>
      )}
    </div>
  )
}

function HfdChart({ samples }: { samples: Sample[] }) {
  if (samples.length === 0) {
    return <div className="h-24 text-xs text-fgdim">collecting…</div>
  }
  const hfds = samples.map((s) => s.hfd)
  const min = Math.min(...hfds)
  const max = Math.max(...hfds)
  const span = Math.max(max - min, 0.5)
  const pts = samples
    .map((s, i) => {
      // a single sample has no span to plot along — pin it to the midpoint
      const x = samples.length > 1 ? (i / (samples.length - 1)) * 100 : 50
      const y = 38 - ((s.hfd - min) / span) * 36
      return `${x.toFixed(2)},${y.toFixed(2)}`
    })
    .join(' ')
  const minY = 38 - ((min - min) / span) * 36
  return (
    <div>
      <svg viewBox="0 0 100 40" preserveAspectRatio="none" className="h-24 w-full">
        <line x1="0" y1={minY} x2="100" y2={minY} stroke="currentColor"
          strokeWidth="0.3" className="text-fgdim" strokeDasharray="2,2" />
        <polyline points={pts} fill="none" stroke="currentColor"
          strokeWidth="0.8" className="text-accent" />
      </svg>
      <div className="flex justify-between text-xs text-fgdim">
        <span>best {min.toFixed(2)}</span>
        <span>{samples.length} samples</span>
      </div>
    </div>
  )
}

export default function FocusPage() {
  const api = getApi()
  const [running, setRunning] = useState(false)
  const [exposureUs, setExposureUs] = useState(DEFAULT_EXPOSURE_US)
  const [gain, setGain] = useState(1)
  const [minExposureUs, setMinExposureUs] = useState(10_000)
  const [gainRange, setGainRange] = useState({ min: 1, max: 16 })
  const [meta, setMeta] = useState<FocusMeta | null>(null)
  const [samples, setSamples] = useState<Sample[]>([])
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const [starUrl, setStarUrl] = useState<string | null>(null)
  const runningRef = useRef(running)
  runningRef.current = running

  useEffect(() => {
    Promise.all([api.getStatus(), api.getSettings()]).then(([s, settings]) => {
      const { gainMin, gainMax } = settings.camera
      setGainRange({ min: gainMin, max: gainMax })
      setRunning(s.focus.enabled)
      setExposureUs(s.focus.exposureUs)
      setGain(nearestOption(s.focus.gain, gainOptions(gainMin, gainMax)))
      if (s.camera) setMinExposureUs(s.camera.minExposureUs)
    })
    return api.subscribe((e) => {
      if (e.type === 'focus') {
        setRunning(true)
        setMeta(e.meta)
        if (e.meta.hfd !== null) {
          setSamples((prev) => [...prev, { t: Date.now(), hfd: e.meta.hfd! }].slice(-MAX_SAMPLES))
        }
        setPreviewUrl(api.focusImageUrl())
        setStarUrl(api.focusStarUrl())
      }
      if (e.type === 'status' && runningRef.current && !e.status.focus.enabled) {
        setRunning(false) // auto-exit on the backend (no viewer / manual stop elsewhere)
      }
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const toggle = async () => {
    const next = !running
    setRunning(next)
    if (!next) {
      setMeta(null)
    } else {
      setSamples([])
    }
    await api.setFocus(next, exposureUs, gain)
  }

  const changeExposure = async (us: number) => {
    setExposureUs(us)
    if (running) await api.setFocus(true, us, gain)
  }

  const changeGain = async (g: number) => {
    setGain(g)
    if (running) await api.setFocus(true, exposureUs, g)
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="mr-auto font-mono text-lg">Focus</h1>
        <label className="flex items-center gap-1.5 text-sm text-fgdim">
          exposure
          <select value={exposureUs} onChange={(e) => changeExposure(Number(e.target.value))}
            className="rounded-lg border border-line bg-panel px-2 py-1.5 text-sm text-fg">
            {exposureOptions(minExposureUs).map((us) => (
              <option key={us} value={us}>{formatExposureUs(us)}</option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1.5 text-sm text-fgdim">
          gain
          <select value={gain} onChange={(e) => changeGain(Number(e.target.value))}
            className="rounded-lg border border-line bg-panel px-2 py-1.5 text-sm text-fg">
            {gainOptions(gainRange.min, gainRange.max).map((g) => (
              <option key={g} value={g}>{g}</option>
            ))}
          </select>
        </label>
        <button onClick={toggle}
          className="rounded-lg bg-accent px-4 py-1.5 text-sm font-medium text-black">
          {running ? 'Stop' : 'Start'}
        </button>
      </div>

      <div className="grid gap-4 md:grid-cols-[2fr_1fr]">
        <div className="rounded-xl border border-line bg-panel p-2">
          {previewUrl ? (
            <ZoomableImage src={previewUrl} alt="focus preview (scroll to zoom, drag to pan)" />
          ) : (
            <div className="flex h-64 items-center justify-center text-sm text-fgdim">
              {running ? 'waiting for the first frame…' : 'press Start to begin focusing'}
            </div>
          )}
        </div>

        <div className="space-y-4">
          <div className="rounded-xl border border-line bg-panel p-4">
            <div className="mb-2 flex items-center justify-between text-sm text-fgdim">
              <span>brightest star</span>
              {meta?.saturated && (
                <span className="rounded bg-red-500/20 px-2 py-0.5 text-xs text-red-400">
                  saturated
                </span>
              )}
            </div>
            {starUrl && (
              <img src={starUrl} alt="brightest star cutout"
                className="mx-auto h-40 w-40 rounded" style={{ imageRendering: 'pixelated' }} />
            )}
            <div className="mt-2 text-center">
              {meta && meta.hfd === null ? (
                <span className="text-sm text-fgdim">no star found</span>
              ) : (
                <>
                  <div className="font-mono text-3xl text-accent">
                    {meta?.hfd?.toFixed(2) ?? '—'}
                  </div>
                  <div className="text-xs text-fgdim">HFD, px — lower is better</div>
                </>
              )}
            </div>
          </div>

          <div className="rounded-xl border border-line bg-panel p-4">
            <div className="mb-1 text-sm text-fgdim">HFD history</div>
            <HfdChart samples={samples} />
          </div>
        </div>
      </div>
    </div>
  )
}
