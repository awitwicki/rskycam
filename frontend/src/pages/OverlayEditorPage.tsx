import { Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type PointerEvent } from 'react'
import { getApi } from '../api/client'
import type {
  CropRect, ImageSettings, LensCalibration, MaskMode, OverlayGeometry,
  OverlaySettings, OverlayTextField, Settings, TextFieldKind,
} from '../api/types'
import { drawOverlay } from '../components/OverlayCanvas'
import { Button, Card, NumberField, Toggle } from '../components/ui'
import {
  applyCropDrag, applyDrag, cropHandlePositions, cropHitTest, handlePositions,
  hitTest, textFieldHitTest, type CropHandle, type EditorHandle, type TextFieldBox,
} from '../lib/editorMath'
import { useStatus } from '../hooks/useStatus'
import { formatExposure, formatGain } from '../lib/format'
import { buildOverlayGeometry } from '../lib/overlayGeometry'

type EditorMode = 'calibrate' | 'crop'
type TextTarget = `text:${string}`
type DragTarget = EditorHandle | CropHandle | TextTarget

function isCropHandle(h: DragTarget): h is CropHandle {
  return h === 'tl' || h === 'br'
}

function isTextTarget(h: DragTarget): h is TextTarget {
  return h.startsWith('text:')
}

function drawSkeleton(ctx: CanvasRenderingContext2D, cal: LensCalibration) {
  ctx.strokeStyle = 'rgba(76,201,240,0.9)'
  ctx.lineWidth = 1.5
  ctx.beginPath()
  ctx.arc(cal.cx, cal.cy, cal.radiusPx, 0, Math.PI * 2)
  ctx.stroke()
  ctx.beginPath()
  ctx.moveTo(cal.cx - 14, cal.cy); ctx.lineTo(cal.cx + 14, cal.cy)
  ctx.moveTo(cal.cx, cal.cy - 14); ctx.lineTo(cal.cx, cal.cy + 14)
  ctx.stroke()
  const hp = handlePositions(cal)
  ctx.fillStyle = 'rgba(76,201,240,1)'
  for (const p of [hp.center, hp.rotation, hp.radius]) {
    ctx.beginPath()
    ctx.arc(p.x, p.y, 8, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.font = '20px ui-monospace, monospace'
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.fillText('N', hp.rotation.x, hp.rotation.y - 20)
  ctx.fillText('E', hp.radius.x + (cal.flip ? -20 : 20), hp.radius.y)
}

/** Draws the draft text fields and returns their hit boxes (drag targets). */
function drawTextFields(
  ctx: CanvasRenderingContext2D,
  fields: OverlayTextField[],
  sampleFor: (kind: TextFieldKind) => string,
): TextFieldBox[] {
  ctx.textAlign = 'left'
  ctx.textBaseline = 'middle'
  const boxes: TextFieldBox[] = []
  for (const f of fields) {
    ctx.font = `${f.fontSize}px ui-monospace, monospace`
    const text = sampleFor(f.kind)
    const width = ctx.measureText(text).width
    ctx.fillStyle = 'rgba(226,232,244,0.95)'
    ctx.fillText(text, f.x, f.y)
    // subtle handle box so fields read as draggable
    ctx.strokeStyle = 'rgba(76,201,240,0.35)'
    ctx.lineWidth = 1
    ctx.setLineDash([4, 4])
    ctx.strokeRect(f.x - 6, f.y - f.fontSize / 2 - 6, width + 12, f.fontSize + 12)
    ctx.setLineDash([])
    boxes.push({ id: f.id, x: f.x, y: f.y, fontSize: f.fontSize, width })
  }
  return boxes
}

/** Dim everything outside the lens circle — preview of maskMode 'circle'. */
function drawMaskPreview(
  ctx: CanvasRenderingContext2D, cal: LensCalibration, w: number, h: number,
) {
  ctx.save()
  ctx.beginPath()
  ctx.rect(0, 0, w, h)
  ctx.arc(cal.cx, cal.cy, cal.radiusPx, 0, Math.PI * 2)
  ctx.clip('evenodd')
  ctx.fillStyle = 'rgba(0,0,0,0.65)'
  ctx.fillRect(0, 0, w, h)
  ctx.restore()
}

function drawCropOverlay(
  ctx: CanvasRenderingContext2D, crop: CropRect, w: number, h: number, active: boolean,
) {
  if (active) {
    ctx.save()
    ctx.beginPath()
    ctx.rect(0, 0, w, h)
    ctx.rect(crop.x, crop.y, crop.width, crop.height)
    ctx.clip('evenodd')
    ctx.fillStyle = 'rgba(0,0,0,0.55)'
    ctx.fillRect(0, 0, w, h)
    ctx.restore()
  }
  ctx.strokeStyle = 'rgba(76,201,240,0.9)'
  ctx.lineWidth = active ? 2 : 1.2
  if (!active) ctx.setLineDash([6, 6])
  ctx.strokeRect(crop.x, crop.y, crop.width, crop.height)
  ctx.setLineDash([])
  if (!active) return
  const hp = cropHandlePositions(crop)
  ctx.fillStyle = 'rgba(76,201,240,1)'
  for (const p of [hp.tl, hp.br]) ctx.fillRect(p.x - 7, p.y - 7, 14, 14)
  ctx.font = '18px ui-monospace, monospace'
  ctx.textAlign = 'left'
  ctx.textBaseline = 'bottom'
  ctx.fillText(`${Math.round(crop.width)}×${Math.round(crop.height)}`, crop.x + 10, crop.y + crop.height - 10)
}

export default function OverlayEditorPage() {
  const [settings, setSettings] = useState<Settings | null>(null)
  const [draft, setDraft] = useState<OverlaySettings | null>(null)
  const [draftImage, setDraftImage] = useState<ImageSettings | null>(null)
  const [mode, setMode] = useState<EditorMode>('calibrate')
  const [dragging, setDragging] = useState<DragTarget | null>(null)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState('')
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const fieldBoxesRef = useRef<TextFieldBox[]>([])
  const grabOffsetRef = useRef({ dx: 0, dy: 0 })
  const { status, frame } = useStatus()
  const [rawUrl, setRawUrl] = useState(() => getApi().latestImageUrl({ raw: true }))
  // Native size of the raw frame, learned when the image loads.
  const [frameDims, setFrameDims] = useState({ w: 1280, h: 960 })

  // The editor always works on the uncropped sensor frame.
  useEffect(() => {
    setRawUrl(getApi().latestImageUrl({ raw: true }))
  }, [frame?.meta.timestamp])

  const sampleFor = (kind: TextFieldKind): string => {
    if (kind === 'time') return new Date().toLocaleString()
    if (kind === 'exposure') {
      const f = status?.capture.lastFrame
      return f ? `exp ${formatExposure(f.exposureUs)} · gain ${formatGain(f.gain)}` : 'exp — · gain —'
    }
    return status?.sensor.reading ? `${status.sensor.reading.temperatureC.toFixed(1)}°C` : '—°C'
  }

  useEffect(() => {
    void getApi().getSettings().then((s) => {
      setSettings(s)
      setDraft(s.overlay)
      setDraftImage(s.image)
    }).catch((e: unknown) => setError(String(e)))
  }, [])

  // Overlay geometry is computed locally from the draft (same math the backend
  // uses) so calibration/layer/opacity edits preview instantly — no round-trip,
  // no waiting for the next frame. Sensor space: crop is not applied here.
  const geometry = useMemo<OverlayGeometry | null>(() => {
    if (!draft || !settings) return null
    return buildOverlayGeometry({
      time: frame ? new Date(frame.meta.timestamp) : new Date(),
      location: settings.location,
      calibration: draft.calibration,
      layers: draft.layers,
      gridOpacity: draft.gridOpacity,
      imageWidth: frameDims.w,
      imageHeight: frameDims.h,
    })
  }, [draft, settings, frameDims, frame?.meta.timestamp]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !draft || !draftImage || !geometry) return
    const w = geometry.imageWidth
    const h = geometry.imageHeight
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, w, h)
    // Fisheye mask dim goes first, as its own opaque layer, so the grid's
    // opacity can never bleed through it. Grid then draws on top.
    if (draftImage.maskMode === 'circle') drawMaskPreview(ctx, draft.calibration, w, h)
    ctx.globalAlpha = 1
    drawOverlay(ctx, { ...geometry, labels: geometry.labels.filter((l) => l.layer !== 'text') })
    if (draftImage.crop) drawCropOverlay(ctx, draftImage.crop, w, h, mode === 'crop')
    if (mode === 'calibrate') {
      drawSkeleton(ctx, draft.calibration)
      fieldBoxesRef.current = drawTextFields(ctx, draft.textFields, sampleFor)
    } else {
      fieldBoxesRef.current = []
    }
  }, [geometry, draft, draftImage, mode, status]) // eslint-disable-line react-hooks/exhaustive-deps

  if (error) return <p className="text-danger">{error}</p>
  if (!draft || !draftImage || !settings) return <p className="text-fgdim">Loading…</p>

  const toImageCoords = (e: PointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    const w = geometry?.imageWidth ?? rect.width
    const h = geometry?.imageHeight ?? rect.height
    return {
      x: ((e.clientX - rect.left) / rect.width) * w,
      y: ((e.clientY - rect.top) / rect.height) * h,
    }
  }

  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    const p = toImageCoords(e)
    let target: DragTarget | null
    if (mode === 'crop') {
      target = draftImage.crop ? cropHitTest(p.x, p.y, draftImage.crop) : null
    } else {
      target = hitTest(p.x, p.y, draft.calibration)
      if (!target) {
        const fieldId = textFieldHitTest(p.x, p.y, fieldBoxesRef.current)
        if (fieldId) {
          const f = draft.textFields.find((x) => x.id === fieldId)
          if (f) {
            grabOffsetRef.current = { dx: p.x - f.x, dy: p.y - f.y }
            target = `text:${fieldId}`
          }
        }
      }
    }
    if (target) {
      setDragging(target)
      e.currentTarget.setPointerCapture(e.pointerId)
    }
  }

  const onPointerMove = (e: PointerEvent<HTMLCanvasElement>) => {
    if (!dragging) return
    const p = toImageCoords(e)
    if (isCropHandle(dragging)) {
      const w = geometry?.imageWidth ?? 1280
      const h = geometry?.imageHeight ?? 960
      setDraftImage((d) => d?.crop
        ? { ...d, crop: applyCropDrag(dragging, p.x, p.y, d.crop, w, h) }
        : d)
    } else if (isTextTarget(dragging)) {
      const { dx, dy } = grabOffsetRef.current
      updateField(dragging.slice(5), {
        x: Math.round(p.x - dx),
        y: Math.round(p.y - dy),
      })
    } else {
      setDraft((d) => d && { ...d, calibration: applyDrag(dragging, p.x, p.y, d.calibration) })
    }
  }

  const setCropEnabled = (on: boolean) => {
    if (on) {
      const w = geometry?.imageWidth ?? 1280
      const h = geometry?.imageHeight ?? 960
      setDraftImage({
        ...draftImage,
        crop: {
          x: Math.round(w * 0.15), y: Math.round(h * 0.15),
          width: Math.round(w * 0.7), height: Math.round(h * 0.7),
        },
      })
      setMode('crop')
    } else {
      setDraftImage({ ...draftImage, crop: null })
      setMode('calibrate')
    }
  }

  const updateField = (id: string, patch: Partial<OverlayTextField>) => {
    setDraft((d) => d && {
      ...d,
      textFields: d.textFields.map((f) => (f.id === id ? { ...f, ...patch } : f)),
    })
  }

  const addField = () => {
    setDraft((d) => d && {
      ...d,
      textFields: [...d.textFields, { id: crypto.randomUUID(), kind: 'time', x: 24, y: 110, fontSize: 18 }],
    })
  }

  const save = async () => {
    const next = { ...settings, overlay: draft, image: draftImage }
    await getApi().putSettings(next)
    setSettings(next)
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const cal = draft.calibration
  const crop = draftImage.crop

  return (
    <div className="grid gap-4 lg:grid-cols-3">
      <Card title="Overlay editor" className="lg:col-span-2">
        <p className="mb-2 text-xs text-fgdim">
          {mode === 'crop'
            ? 'Drag the corner handles to crop the frame. Everything dimmed is cut away.'
            : 'Drag the center crosshair, the E handle (radius), the N handle (rotation) — or grab a text label to reposition it. The full sensor frame is shown here; the dashboard shows the cropped result.'}
        </p>
        <div className="overflow-hidden rounded-lg bg-night">
          <div className="relative lg:mx-auto lg:w-fit">
            <img src={rawUrl} alt="Calibration frame"
              onLoad={(e) => {
                const img = e.currentTarget
                if (img.naturalWidth > 0) setFrameDims({ w: img.naturalWidth, h: img.naturalHeight })
              }}
              className="w-full lg:h-auto lg:max-h-[calc(100dvh-16rem)] lg:w-auto lg:max-w-full" />
            <canvas
              ref={canvasRef}
              className="absolute inset-0 h-full w-full touch-none cursor-crosshair"
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={() => setDragging(null)}
              onPointerCancel={() => setDragging(null)}
            />
          </div>
        </div>
      </Card>

      <div className="flex flex-col gap-4">
        <Card title="Image">
          <div className="flex flex-col gap-3">
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-fgdim">Mask</span>
              <select value={draftImage.maskMode}
                onChange={(e) => setDraftImage({ ...draftImage, maskMode: e.target.value as MaskMode })}
                className="rounded-lg border border-line bg-panel2 px-3 py-2 text-fg">
                <option value="none">Full frame (no mask)</option>
                <option value="circle">Fisheye circle (black mask)</option>
              </select>
            </label>
            <Toggle label="Crop frame" checked={crop !== null} onChange={setCropEnabled} />
            {crop && (
              <>
                <div className="flex gap-2">
                  <Button variant={mode === 'calibrate' ? 'primary' : 'ghost'}
                    onClick={() => setMode('calibrate')} className="flex-1 !py-1.5 text-xs">
                    Calibrate
                  </Button>
                  <Button variant={mode === 'crop' ? 'primary' : 'ghost'}
                    onClick={() => setMode('crop')} className="flex-1 !py-1.5 text-xs">
                    Edit crop
                  </Button>
                </div>
                <p className="font-mono text-xs text-fgdim">
                  crop {Math.round(crop.x)},{Math.round(crop.y)} · {Math.round(crop.width)}×{Math.round(crop.height)} px
                </p>
              </>
            )}
          </div>
        </Card>

        <Card title="Layers">
          <div className="flex flex-col gap-3">
            <Toggle label="Cardinal directions" checked={draft.layers.cardinal}
              onChange={(v) => setDraft({ ...draft, layers: { ...draft.layers, cardinal: v } })} />
            <Toggle label="Alt/Az grid" checked={draft.layers.altAzGrid}
              onChange={(v) => setDraft({ ...draft, layers: { ...draft.layers, altAzGrid: v } })} />
            <Toggle label="RA/Dec grid" checked={draft.layers.raDecGrid}
              onChange={(v) => setDraft({ ...draft, layers: { ...draft.layers, raDecGrid: v } })} />
            <label className="flex flex-col gap-1 text-sm">
              <span className="flex items-baseline justify-between">
                <span className="text-fgdim">Grid opacity</span>
                <span className="font-mono text-xs">{Math.round(draft.gridOpacity * 100)}%</span>
              </span>
              <input type="range" min={0.1} max={1} step={0.05} value={draft.gridOpacity}
                aria-label="Grid opacity"
                onChange={(e) => setDraft({ ...draft, gridOpacity: Number(e.target.value) })}
                className="accent-accent" />
            </label>
            <Toggle label="Mirror east/west (flip)" checked={cal.flip}
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, flip: v } })} />
            <Toggle label="Bake overlay into saved frames" checked={draft.bakeIntoSavedFrames}
              onChange={(v) => setDraft({ ...draft, bakeIntoSavedFrames: v })} />
          </div>
        </Card>

        <Card title="Calibration">
          <div className="grid grid-cols-2 gap-3">
            <NumberField label="Center X" value={Math.round(cal.cx)}
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, cx: v } })} suffix="px" />
            <NumberField label="Center Y" value={Math.round(cal.cy)}
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, cy: v } })} suffix="px" />
            <NumberField label="Radius" value={Math.round(cal.radiusPx)}
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, radiusPx: v } })} suffix="px" />
            <NumberField label="Rotation" value={Math.round(cal.rotationDeg)}
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, rotationDeg: v } })} suffix="°" />
          </div>
        </Card>

        <Card title="Text fields"
          action={
            <Button variant="ghost" onClick={addField} className="!px-2 !py-1 text-xs">
              <Plus size={12} /> Add
            </Button>
          }>
          <div className="flex flex-col gap-3">
            {draft.textFields.map((f) => (
              <div key={f.id} className="rounded-lg border border-line p-2">
                <div className="mb-2 flex items-center justify-between">
                  <select value={f.kind}
                    onChange={(e) => updateField(f.id, { kind: e.target.value as TextFieldKind })}
                    className="rounded border border-line bg-panel2 px-2 py-1 text-xs text-fg">
                    <option value="time">Frame time</option>
                    <option value="exposure">Exposure / gain</option>
                    <option value="sensorTemp">Sensor temperature</option>
                  </select>
                  <button aria-label={`Remove ${f.kind} field`}
                    onClick={() => setDraft({ ...draft, textFields: draft.textFields.filter((x) => x.id !== f.id) })}
                    className="text-fgdim hover:text-danger">
                    <Trash2 size={14} />
                  </button>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  <NumberField label="X" value={f.x} onChange={(v) => updateField(f.id, { x: v })} />
                  <NumberField label="Y" value={f.y} onChange={(v) => updateField(f.id, { y: v })} />
                  <NumberField label="Size" value={f.fontSize}
                    onChange={(v) => updateField(f.id, { fontSize: v })} />
                </div>
              </div>
            ))}
            {draft.textFields.length === 0 && (
              <p className="text-xs text-fgdim">No text fields — add one above.</p>
            )}
          </div>
        </Card>

        <div className="flex items-center gap-3">
          <Button onClick={save}>Save</Button>
          {saved && <span className="text-sm text-ok">Saved ✓</span>}
        </div>
      </div>
    </div>
  )
}
