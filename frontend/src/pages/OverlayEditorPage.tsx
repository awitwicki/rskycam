import { Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type PointerEvent } from 'react'
import { getApi } from '../api/client'
import type {
  CropRect, ImageSettings, LensCalibration, LensType, MaskMode, OverlayGeometry,
  OverlayLayers, OverlaySettings, OverlayTextField, Settings, TextFieldKind,
} from '../api/types'
import { drawOverlay } from '../components/OverlayCanvas'
import { Button, Card, NumberField, Toggle } from '../components/ui'
import {
  applyCenterPan, applyCropDrag, applyMaskDrag, applyRollDrag, applySkyPan,
  applyWheelZoom, calibrationHitTest, cropHandlePositions, cropHitTest,
  imageToAltAz, maskHandlePositions, maskHitTest, rollHandlePosition,
  textFieldHitTest,
  type CalibrationTarget, type CropHandle, type MaskHandle, type TextFieldBox,
} from '../lib/editorMath'
import { focalLengthPx, opticalCenter, type LensView } from '../lib/astro'
import { useStatus } from '../hooks/useStatus'
import { formatExposure, formatGain } from '../lib/format'
import { buildOverlayGeometry } from '../lib/overlayGeometry'
import { uid } from '../lib/uid'

type EditorMode = 'calibrate' | 'crop'
type TextTarget = `text:${string}`
type DragTarget = CalibrationTarget | MaskHandle | CropHandle | TextTarget

function isCropHandle(h: DragTarget): h is CropHandle {
  return h === 'tl' || h === 'br'
}

function isMaskHandle(h: DragTarget): h is MaskHandle {
  return h === 'maskCenter' || h === 'maskRadius'
}

function isTextTarget(h: DragTarget): h is TextTarget {
  return h.startsWith('text:')
}

/** With every layer off there is nothing on screen to calibrate — the
 *  Calibration card, skeleton and aim/zoom gestures all hide together. */
function anyLayerOn(layers: OverlayLayers): boolean {
  return layers.cardinal || layers.altAzGrid || layers.raDecGrid
}

function drawSkeleton(
  ctx: CanvasRenderingContext2D, cal: LensCalibration, view: LensView, zenithMode: boolean,
) {
  const oc = opticalCenter(cal, view)
  const fPx = focalLengthPx(cal, view)
  const r45 = cal.lensType === 'fisheye' ? (fPx * Math.PI) / 4 : fPx
  ctx.strokeStyle = 'rgba(76,201,240,0.9)'
  ctx.lineWidth = 1.5
  ctx.beginPath()
  ctx.arc(oc.x, oc.y, r45, 0, Math.PI * 2)
  ctx.stroke()
  ctx.beginPath()
  ctx.moveTo(oc.x - 14, oc.y); ctx.lineTo(oc.x + 14, oc.y)
  ctx.moveTo(oc.x, oc.y - 14); ctx.lineTo(oc.x, oc.y + 14)
  ctx.stroke()
  // The rotation orbit + N handle exist only in zenith (all-sky) mode, where
  // the handle's position truly marks sky-north. A tilted camera is always
  // level (zenith at image-top), so there is nothing to rotate by hand.
  if (!zenithMode) return
  const R = 0.35 * Math.min(view.frameWidth, view.frameHeight)
  ctx.setLineDash([4, 6])
  ctx.beginPath()
  ctx.arc(oc.x, oc.y, R, 0, Math.PI * 2)
  ctx.stroke()
  ctx.setLineDash([])
  const h = rollHandlePosition(cal, view)
  ctx.fillStyle = 'rgba(76,201,240,1)'
  ctx.beginPath()
  ctx.arc(h.x, h.y, 8, 0, Math.PI * 2)
  ctx.fill()
  ctx.font = '20px ui-monospace, monospace'
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.fillText('N', h.x, h.y - 20)
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

/** Dim everything outside the manual mask circle — preview of maskMode 'circle'. */
function drawMaskPreview(
  ctx: CanvasRenderingContext2D, image: ImageSettings, w: number, h: number,
) {
  ctx.save()
  ctx.beginPath()
  ctx.rect(0, 0, w, h)
  ctx.arc(image.maskCenterXPx, image.maskCenterYPx, image.maskRadiusPx, 0, Math.PI * 2)
  ctx.clip('evenodd')
  ctx.fillStyle = 'rgba(0,0,0,0.65)'
  ctx.fillRect(0, 0, w, h)
  ctx.restore()
}

/** Draggable outline + handles for the manual mask circle (calibrate mode). */
function drawMaskHandles(ctx: CanvasRenderingContext2D, image: ImageSettings) {
  ctx.strokeStyle = 'rgba(255,193,94,0.9)'
  ctx.lineWidth = 1.5
  ctx.beginPath()
  ctx.arc(image.maskCenterXPx, image.maskCenterYPx, image.maskRadiusPx, 0, Math.PI * 2)
  ctx.stroke()
  const hp = maskHandlePositions(image)
  ctx.fillStyle = 'rgba(255,193,94,1)'
  for (const p of [hp.maskCenter, hp.maskRadius]) {
    ctx.beginPath()
    ctx.arc(p.x, p.y, 8, 0, Math.PI * 2)
    ctx.fill()
  }
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
  // Zenith (all-sky) vs tilted camera — a UI mode, never persisted: derived
  // from the loaded pointing and re-derivable from what gets saved.
  const [zenithMode, setZenithMode] = useState(true)
  const [dragging, setDragging] = useState<DragTarget | null>(null)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState('')
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const fieldBoxesRef = useRef<TextFieldBox[]>([])
  const grabOffsetRef = useRef({ dx: 0, dy: 0 })
  const panGrabRef = useRef<{ altDeg: number; azDeg: number } | null>(null)
  const centerGrabRef = useRef<{ offsetX: number; offsetY: number; x: number; y: number } | null>(null)
  const { status, frame } = useStatus()
  const [rawUrl, setRawUrl] = useState(() => getApi().latestImageUrl({ raw: true }))
  // Native size of the raw frame, learned when the image loads.
  const [frameDims, setFrameDims] = useState({ w: 1280, h: 960 })

  const view: LensView = useMemo(() => ({
    frameWidth: frameDims.w,
    frameHeight: frameDims.h,
    nativeWidth: status?.camera?.maxWidth ?? frameDims.w,
  }), [frameDims, status?.camera?.maxWidth])

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
      const c = s.overlay.calibration
      const zen = c.pointingAltDeg === 90 && c.pointingAzDeg === 0
      setZenithMode(zen)
      // A tilted camera is always level (zenith at image-top = roll 180 in
      // the internal convention); normalize a stray roll so the hidden value
      // can't invisibly rotate the grid.
      const overlay = zen || c.rollDeg === 180
        ? s.overlay
        : { ...s.overlay, calibration: { ...c, rollDeg: 180 } }
      setSettings(s)
      setDraft(overlay)
      setDraftImage(s.image)
    }).catch((e: unknown) => setError(String(e)))
  }, [])

  const setZenith = (on: boolean) => {
    setZenithMode(on)
    setDraft((d) => d && {
      ...d,
      calibration: on
        ? { ...d.calibration, pointingAltDeg: 90, pointingAzDeg: 0 }
        : { ...d.calibration, rollDeg: 180 }, // tilted = level, zenith at top
    })
  }

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
      nativeWidth: view.nativeWidth,
      mask: draftImage?.maskMode === 'circle'
        ? {
            centerXPx: draftImage.maskCenterXPx,
            centerYPx: draftImage.maskCenterYPx,
            radiusPx: draftImage.maskRadiusPx,
          }
        : undefined,
    })
  }, [draft, draftImage, settings, frameDims, frame?.meta.timestamp, view]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !draft || !draftImage || !geometry) return
    const w = geometry.imageWidth
    const h = geometry.imageHeight
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, w, h)
    // Fisheye mask dim goes first, as its own opaque layer. The grid can't
    // cross it anyway: its geometry is culled to the mask circle at build
    // time (the mask covers the grid), matching the backend bake exactly.
    if (draftImage.maskMode === 'circle') {
      drawMaskPreview(ctx, draftImage, w, h)
    }
    ctx.globalAlpha = 1
    drawOverlay(ctx, { ...geometry, labels: geometry.labels.filter((l) => l.layer !== 'text') })
    if (draftImage.crop) drawCropOverlay(ctx, draftImage.crop, w, h, mode === 'crop')
    if (mode === 'calibrate') {
      if (anyLayerOn(draft.layers)) drawSkeleton(ctx, draft.calibration, view, zenithMode)
      if (draftImage.maskMode === 'circle') drawMaskHandles(ctx, draftImage)
      fieldBoxesRef.current = drawTextFields(ctx, draft.textFields, sampleFor)
    } else {
      fieldBoxesRef.current = []
    }
  }, [geometry, draft, draftImage, mode, status, view, zenithMode]) // eslint-disable-line react-hooks/exhaustive-deps

  // Canvas only exists in the DOM once the initial load finishes (the
  // component renders a "Loading…" placeholder, with no canvas, until then).
  // `loaded` is a stable boolean — unlike the `draft` object, it doesn't
  // change identity on every drag-driven update — that flips false -> true
  // exactly once, so the wheel-zoom effect below re-runs right when the
  // canvas mounts and finds a non-null canvasRef.
  const loaded = draft !== null
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const onWheel = (e: WheelEvent) => {
      if (mode !== 'calibrate') return
      e.preventDefault()
      setDraft((d) => (d && anyLayerOn(d.layers))
        ? { ...d, calibration: applyWheelZoom(d.calibration, e.deltaY) }
        : d)
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => canvas.removeEventListener('wheel', onWheel)
  }, [mode, loaded])

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

  /** Text-field grab, or null. Side effect: records the grab offset. */
  const hitTextField = (p: { x: number; y: number }): DragTarget | null => {
    const fieldId = textFieldHitTest(p.x, p.y, fieldBoxesRef.current)
    const f = fieldId ? draft.textFields.find((x) => x.id === fieldId) : undefined
    if (!f) return null
    grabOffsetRef.current = { dx: p.x - f.x, dy: p.y - f.y }
    return `text:${f.id}`
  }

  /** Grid pan grab: center offsets in zenith mode, sky alt/az when tilted. */
  const startGridPan = (p: { x: number; y: number }): DragTarget => {
    if (zenithMode) {
      centerGrabRef.current = {
        offsetX: draft.calibration.centerOffsetXPx,
        offsetY: draft.calibration.centerOffsetYPx,
        x: p.x,
        y: p.y,
      }
    } else {
      panGrabRef.current = imageToAltAz(p.x, p.y, draft.calibration, view)
    }
    return 'pan'
  }

  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    const p = toImageCoords(e)
    let target: DragTarget | null
    if (mode === 'crop') {
      target = draftImage.crop ? cropHitTest(p.x, p.y, draftImage.crop) : null
    } else {
      // The N rotation handle exists only in zenith mode. With all layers
      // off the grid is invisible — aiming gestures are disabled so a stray
      // drag can't silently corrupt the calibration.
      const calibrating = anyLayerOn(draft.layers)
      target = (calibrating && zenithMode ? calibrationHitTest(p.x, p.y, draft.calibration, view) : null)
        ?? (draftImage.maskMode === 'circle' ? maskHitTest(p.x, p.y, draftImage) : null)
        ?? hitTextField(p)
        ?? (calibrating ? startGridPan(p) : null)
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
    } else if (isMaskHandle(dragging)) {
      setDraftImage((d) => d && applyMaskDrag(dragging, p.x, p.y, d))
    } else if (isTextTarget(dragging)) {
      const { dx, dy } = grabOffsetRef.current
      updateField(dragging.slice(5), {
        x: Math.round(p.x - dx),
        y: Math.round(p.y - dy),
      })
    } else if (dragging === 'roll') {
      setDraft((d) => d && { ...d, calibration: applyRollDrag(p.x, p.y, d.calibration, view) })
    } else if (zenithMode) {
      const grab = centerGrabRef.current
      if (grab) {
        setDraft((d) => d && { ...d, calibration: applyCenterPan(grab, p.x, p.y, d.calibration) })
      }
    } else {
      const grab = panGrabRef.current
      if (grab) {
        setDraft((d) => d && { ...d, calibration: applySkyPan(grab, p.x, p.y, d.calibration, view) })
      }
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
      textFields: [...d.textFields, { id: uid(), kind: 'time', x: 24, y: 110, fontSize: 18 }],
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

  let editorHint: string
  if (mode === 'crop') {
    editorHint = 'Drag the corner handles to crop the frame. Everything dimmed is cut away.'
  } else if (zenithMode) {
    editorHint = 'Drag anywhere to center the grid, drag the N handle to rotate it, scroll to zoom — or grab a text label to reposition it. The full sensor frame is shown here; the dashboard shows the cropped result.'
  } else {
    editorHint = 'Drag anywhere to aim the grid (the camera is assumed level, zenith up), scroll to zoom — or grab a text label to reposition it. The full sensor frame is shown here; the dashboard shows the cropped result.'
  }

  return (
    <div className="grid gap-4 lg:grid-cols-3">
      <Card title="Overlay editor" className="lg:col-span-2">
        <p className="mb-2 text-xs text-fgdim">
          {editorHint}
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
            {draftImage.maskMode === 'circle' && (
              <>
                <div className="grid grid-cols-3 gap-2">
                  <NumberField label="Center X" value={Math.round(draftImage.maskCenterXPx)}
                    onChange={(v) => setDraftImage({ ...draftImage, maskCenterXPx: v })} />
                  <NumberField label="Center Y" value={Math.round(draftImage.maskCenterYPx)}
                    onChange={(v) => setDraftImage({ ...draftImage, maskCenterYPx: v })} />
                  <NumberField label="Radius" value={Math.round(draftImage.maskRadiusPx)}
                    min={20} onChange={(v) => setDraftImage({ ...draftImage, maskRadiusPx: Math.max(20, v) })} />
                </div>
                <p className="text-xs text-fgdim">
                  Manual circle in sensor pixels — drag the orange center/edge handles in
                  the preview, or type values here.
                </p>
              </>
            )}
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

        {anyLayerOn(draft.layers) && <Card title="Calibration">
          <div className="mb-3">
            <Toggle label="Camera points at the zenith (all-sky)" checked={zenithMode}
              onChange={setZenith} />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <label className="col-span-2 flex flex-col gap-1 text-sm">
              <span className="text-fgdim">Lens type</span>
              <select value={cal.lensType}
                onChange={(e) => setDraft({ ...draft, calibration: { ...cal, lensType: e.target.value as LensType } })}
                className="rounded-lg border border-line bg-panel2 px-3 py-2 text-fg">
                <option value="fisheye">Fisheye (r = f·θ)</option>
                <option value="rectilinear">Rectilinear (r = f·tan θ)</option>
              </select>
            </label>
            <NumberField label="Focal length" value={Number(cal.focalLengthMm.toFixed(2))}
              step={0.1} min={0.1} max={100} suffix="mm"
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, focalLengthMm: v } })} />
            <NumberField label="Pixel size" value={Number(cal.pixelSizeUm.toFixed(2))}
              step={0.01} min={0.5} max={50} suffix="µm"
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, pixelSizeUm: v } })} />
            {zenithMode ? (
              <NumberField label="Rotation (north)" value={Math.round(cal.rollDeg * 10) / 10}
                step={0.5} suffix="°"
                onChange={(v) => setDraft({ ...draft, calibration: { ...cal, rollDeg: ((v % 360) + 360) % 360 } })} />
            ) : (
              <>
                <NumberField label="Pointing azimuth" value={Math.round(cal.pointingAzDeg * 10) / 10}
                  step={0.5} suffix="°"
                  onChange={(v) => setDraft({ ...draft, calibration: { ...cal, pointingAzDeg: ((v % 360) + 360) % 360 } })} />
                <NumberField label="Pointing altitude" value={Math.round(cal.pointingAltDeg * 10) / 10}
                  step={0.5} min={-90} max={90} suffix="°"
                  onChange={(v) => setDraft({ ...draft, calibration: { ...cal, pointingAltDeg: Math.min(90, Math.max(-90, v)) } })} />
              </>
            )}
            <NumberField label="Offset X" value={Math.round(cal.centerOffsetXPx)} suffix="px"
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, centerOffsetXPx: v } })} />
            <NumberField label="Offset Y" value={Math.round(cal.centerOffsetYPx)} suffix="px"
              onChange={(v) => setDraft({ ...draft, calibration: { ...cal, centerOffsetYPx: v } })} />
          </div>
          <p className="mt-2 text-xs text-fgdim">
            {zenithMode
              ? 'Pixel size is the sensor’s native value (imx219: 1.12 µm); binning is derived automatically. Drag the image to center the grid, drag the N handle to rotate it, scroll to fine-tune the focal length.'
              : 'The tilted camera is assumed level — the zenith is always toward the image top. Drag the image to aim the grid (azimuth/altitude), scroll to fine-tune the focal length.'}
          </p>
        </Card>}

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
                    <option value="sensorTemp">Outdoor temp (BME280)</option>
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

        {/* Sticky so Save stays reachable however long the sidebar grows;
            on mobile it rides above the fixed bottom tab bar. */}
        <div className="sticky bottom-14 -my-2 flex items-center gap-3 bg-night py-2 md:bottom-0">
          <Button onClick={save}>Save</Button>
          {saved && <span className="text-sm text-ok">Saved ✓</span>}
        </div>
      </div>
    </div>
  )
}
