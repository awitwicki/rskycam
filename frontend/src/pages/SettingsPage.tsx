import { Download, Map as MapIcon, Upload } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { getApi } from '../api/client'
import type { DarksLibrary, Settings } from '../api/types'
import LocationPicker from '../components/LocationPicker'
import { Button, Card, Input, NumberField, Toggle } from '../components/ui'
import { useStatus } from '../hooks/useStatus'
import { formatExposure, formatGain } from '../lib/format'

// Every settings section a valid export must carry — a light structural
// check so importing an unrelated JSON file fails with a clear error
// instead of silently loading a broken draft (missing sections would render
// as blank/NaN fields throughout the form).
const SETTINGS_SECTIONS: (keyof Settings)[] = [
  'camera', 'image', 'location', 'sensor', 'overlay', 'processing', 'storage', 'darks',
]
function isSettingsShape(v: unknown): v is Settings {
  return !!v && typeof v === 'object' && SETTINGS_SECTIONS.every((k) => k in (v as object))
}

export default function SettingsPage() {
  const { status } = useStatus()
  const [draft, setDraft] = useState<Settings | null>(null)
  const [showMap, setShowMap] = useState(false)
  const [saved, setSaved] = useState(false)
  const [oldPw, setOldPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [pwMessage, setPwMessage] = useState<{ ok: boolean; text: string } | null>(null)
  const [error, setError] = useState('')
  const [importError, setImportError] = useState('')
  const importInputRef = useRef<HTMLInputElement>(null)
  const [confirmSweep, setConfirmSweep] = useState(false)
  const [startingSweep, setStartingSweep] = useState(false)
  const [darksLibrary, setDarksLibrary] = useState<DarksLibrary | null>(null)
  const [darksError, setDarksError] = useState('')

  useEffect(() => {
    void getApi().getSettings().then(setDraft).catch((e: unknown) => setError(String(e)))
  }, [])

  useEffect(() => {
    void getApi().getDarksLibrary().then(setDarksLibrary)
  }, [status])

  if (error) return <p className="text-danger">{error}</p>
  if (!draft) return <p className="text-fgdim">Loading…</p>

  const patch = <K extends keyof Settings>(key: K, value: Partial<Settings[K]>) =>
    setDraft((d) => d && { ...d, [key]: { ...d[key], ...value } })

  const exportSettings = () => {
    const blob = new Blob([JSON.stringify(draft, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'rskycam-settings.json'
    a.click()
    URL.revokeObjectURL(url)
  }

  const importSettings = async (file: File) => {
    setImportError('')
    try {
      const parsed: unknown = JSON.parse(await file.text())
      if (!isSettingsShape(parsed)) throw new Error('not a valid rskycam settings file')
      setDraft(parsed) // loaded into the draft, not yet saved — review, then Save settings
    } catch (e: unknown) {
      setImportError(`Import failed: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const save = async () => {
    await getApi().putSettings(draft)
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const changePassword = async () => {
    const ok = await getApi().changePassword(oldPw, newPw)
    setPwMessage(ok
      ? { ok: true, text: 'Password changed' }
      : { ok: false, text: 'Current password is incorrect' })
    if (ok) {
      setOldPw('')
      setNewPw('')
    }
  }

  const startSweep = async () => {
    setConfirmSweep(false)
    setStartingSweep(true)
    setDarksError('')
    try {
      await getApi().startDarksCapture()
    } catch (e: unknown) {
      setDarksError(String(e))
    } finally {
      setStartingSweep(false)
    }
  }

  const clearDarks = async () => {
    await getApi().clearDarks()
    setDarksLibrary(await getApi().getDarksLibrary())
  }

  const cam = draft.camera
  const live = status?.sensor

  // Candidate presets, largest first; filtered to what the connected camera's
  // sensor supports (status.camera), so we never offer a resolution the driver
  // would silently clamp. Falls back to all presets when caps are unknown.
  const RES_PRESETS: [number, number, string][] = [
    [3280, 2464, '8 MP'],
    [1640, 1232, '2 MP'],
    [1280, 960, '1.2 MP'],
    [960, 720, '0.7 MP'],
    [800, 600, '0.5 MP'],
    [640, 480, '0.3 MP'],
  ]
  const caps = status?.camera ?? null
  const maxW = caps?.maxWidth ?? Infinity
  const maxH = caps?.maxHeight ?? Infinity
  const resOptions = RES_PRESETS.filter(([w, h]) => w <= maxW && h <= maxH)
  // Always offer the sensor's exact native size, and the currently-saved value
  // even if it now exceeds the sensor (so the <select> can show it).
  const ensure = (w: number, h: number, label: string) => {
    if (!resOptions.some(([ow, oh]) => ow === w && oh === h)) resOptions.unshift([w, h, label])
  }
  if (caps) ensure(caps.maxWidth, caps.maxHeight, 'native')
  ensure(cam.captureWidth, cam.captureHeight, 'current')

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-4">
      {/* Sticky so Save stays reachable however far the page scrolls. */}
      <header className="sticky top-0 z-10 -my-2 flex items-center justify-between bg-night py-2">
        <h1 className="text-lg font-medium">Settings</h1>
        <div className="flex items-center gap-3">
          {saved && <span className="text-sm text-ok">Saved ✓</span>}
          <input ref={importInputRef} type="file" accept="application/json,.json" className="hidden"
            onChange={(e) => {
              const file = e.target.files?.[0]
              if (file) void importSettings(file)
              e.target.value = '' // allow re-importing the same file later
            }} />
          <Button variant="ghost" onClick={() => importInputRef.current?.click()}>
            <Upload size={14} /> Import
          </Button>
          <Button variant="ghost" onClick={exportSettings}>
            <Download size={14} /> Export
          </Button>
          <Button onClick={save}>Save settings</Button>
        </div>
      </header>
      {importError && <p className="text-sm text-danger">{importError}</p>}

      <Card title="Camera">
        <div className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-3">
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-fgdim">Driver</span>
              <select value={cam.driver}
                onChange={(e) => patch('camera', { driver: e.target.value as 'asi' | 'rpicam' | 'mock' })}
                className="rounded-lg border border-line bg-panel2 px-3 py-2 text-fg">
                <option value="rpicam">Raspberry Pi camera (CSI)</option>
                <option value="mock">Mock (synthetic sky)</option>
                <option value="asi">ZWO ASI (USB)</option>
              </select>
            </label>
            <NumberField label="Day capture interval" value={cam.intervalSecDay}
              onChange={(v) => patch('camera', { intervalSecDay: v })} suffix="s" min={0} />
            <NumberField label="Night capture interval" value={cam.intervalSecNight}
              onChange={(v) => patch('camera', { intervalSecNight: v })} suffix="s" min={0} />
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-fgdim">Resolution</span>
              <select value={`${cam.captureWidth}x${cam.captureHeight}`}
                onChange={(e) => {
                  const [w, h] = e.target.value.split('x').map(Number)
                  patch('camera', { captureWidth: w, captureHeight: h })
                }}
                className="rounded-lg border border-line bg-panel2 px-3 py-2 text-fg">
                {resOptions.map(([w, h, label]) => (
                  <option key={`${w}x${h}`} value={`${w}x${h}`}>
                    {w}×{h} — {label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="text-xs text-fgdim">
            Interval 0 = continuous shooting: captures back-to-back with no gap, once exposure has settled.
          </p>
          <p className="text-xs text-fgdim">
            {caps
              ? `Resolutions available on ${caps.model} (max ${caps.maxWidth}×${caps.maxHeight}).`
              : 'Smaller resolutions save disk and CPU.'}
          </p>
          <Toggle label="Auto exposure" checked={cam.autoExposure}
            onChange={(v) => patch('camera', { autoExposure: v })} />
          {cam.autoExposure ? (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              <NumberField label="Target brightness" value={cam.targetBrightness}
                onChange={(v) => patch('camera', { targetBrightness: v })} min={0} max={255} />
              <NumberField label="Min exposure" value={cam.exposureUsMin}
                onChange={(v) => patch('camera', { exposureUsMin: v })} suffix="µs" min={1} />
              <NumberField label="Max exposure" value={cam.exposureUsMax / 1e6}
                onChange={(v) => patch('camera', { exposureUsMax: v * 1e6 })} suffix="s" min={1} />
              <NumberField label="Min gain" value={cam.gainMin}
                onChange={(v) => patch('camera', { gainMin: v })} min={0} />
              <NumberField label="Max gain" value={cam.gainMax}
                onChange={(v) => patch('camera', { gainMax: v })} min={0} />
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-3">
              <NumberField label="Exposure" value={cam.manualExposureUs / 1e6}
                onChange={(v) => patch('camera', { manualExposureUs: v * 1e6 })} suffix="s" min={0} />
              <NumberField label="Gain" value={cam.manualGain}
                onChange={(v) => patch('camera', { manualGain: v })} min={0} />
            </div>
          )}
          <Toggle label="Capture during the day" checked={cam.captureDuringDay}
            onChange={(v) => patch('camera', { captureDuringDay: v })} />
        </div>
      </Card>

      <Card title="Darks">
        <div className="flex flex-col gap-4">
          <Toggle label="Use dark-frame correction" checked={draft.darks.enabled}
            onChange={(v) => patch('darks', { enabled: v })} />
          <div className="grid grid-cols-2 gap-3">
            <NumberField label="Min gain to apply" value={draft.darks.minGainToApply}
              onChange={(v) => patch('darks', { minGainToApply: v })} min={0} />
            <NumberField label="Min exposure to apply" value={draft.darks.minExposureUsToApply / 1e6}
              onChange={(v) => patch('darks', { minExposureUsToApply: v * 1e6 })} suffix="s" min={0} />
          </div>
          <p className="text-xs text-fgdim">
            Subtracts the closest-matching dark frame to remove hot pixels, only above both thresholds.
          </p>
          <div className="flex items-center gap-3">
            <Button variant="ghost" onClick={() => setConfirmSweep(true)}
              disabled={startingSweep || !!status?.darksProgress ||
                status?.capture.state === 'camera_unavailable'}>
              Capture dark sweep
            </Button>
            {status?.darksProgress && (
              <span className="text-sm text-fgdim">
                Capturing {status.darksProgress.current}/{status.darksProgress.total}…
              </span>
            )}
          </div>
          {status?.darksProgress && (
            <div className="h-1.5 rounded bg-panel2">
              <div className="h-full rounded bg-accent"
                style={{ width: `${(100 * status.darksProgress.current) / status.darksProgress.total}%` }} />
            </div>
          )}
          {darksError && <p className="text-sm text-danger">{darksError}</p>}
          <div>
            <h3 className="text-xs font-medium uppercase tracking-wider text-fgdim">Captured darks</h3>
            {darksLibrary && darksLibrary.entries.length > 0 ? (
              <ul className="mt-2 flex flex-col gap-1 text-sm text-fgdim">
                {darksLibrary.entries.map((e) => (
                  <li key={e.file}>
                    {formatExposure(e.exposureUs)} · gain {formatGain(e.gain)} ·{' '}
                    {new Date(e.capturedAt).toLocaleString()}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-2 text-sm text-fgdim">No darks captured yet.</p>
            )}
            {darksLibrary && darksLibrary.entries.length > 0 && (
              <Button variant="ghost" className="mt-2 text-danger hover:border-danger" onClick={clearDarks}>
                Clear darks
              </Button>
            )}
          </div>
        </div>
      </Card>

      {confirmSweep && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-night/80 p-4"
          role="dialog" aria-modal="true" aria-label="Confirm dark sweep"
          onClick={() => setConfirmSweep(false)}>
          <div className="w-full max-w-sm rounded-xl border border-line bg-panel p-5"
            onClick={(e) => e.stopPropagation()}>
            <h2 className="font-mono text-base">Capture dark sweep?</h2>
            <p className="mt-2 text-sm text-fgdim">
              Cover the lens completely before starting — no light should reach the sensor.
              Only exposure/gain points above the apply thresholds are swept,
              several frames stacked into a master dark each — usually a few
              minutes. Pauses normal capture.
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirmSweep(false)}>Cancel</Button>
              <Button variant="primary" onClick={startSweep}>Start sweep</Button>
            </div>
          </div>
        </div>
      )}

      <Card title="Sensor">
        <div className="flex flex-col gap-3">
          <Toggle label="BME280 / BMP280 sensor (I2C)" checked={draft.sensor.enabled}
            onChange={(v) => patch('sensor', { enabled: v })} />
          {live && (
            live.reading ? (
              <p className="text-sm text-ok">
                ● Detected — {live.reading.temperatureC.toFixed(1)}°C
                · {Math.round(live.reading.pressureHpa)} hPa
              </p>
            ) : live.state === 'not_detected' ? (
              <p className="text-sm text-danger">
                ● Not detected — check the I2C wiring and address (0x76 / 0x77)
              </p>
            ) : (
              <p className="text-sm text-fgdim">● Disabled</p>
            )
          )}
          {live && (live.state !== 'disabled') !== draft.sensor.enabled && (
            <p className="text-xs text-warn">Save settings to apply.</p>
          )}
          <p className="text-xs text-fgdim">
            Readings appear on the dashboard; temperature is available as an overlay text field.
          </p>
        </div>
      </Card>

      <Card title="Location"
        action={
          <Button variant="ghost" onClick={() => setShowMap((v) => !v)}>
            <MapIcon size={14} /> {showMap ? 'Hide map' : 'Pick on map'}
          </Button>
        }>
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <NumberField label="Latitude" value={draft.location.latitudeDeg}
              onChange={(v) => patch('location', { latitudeDeg: v })} suffix="°" step={0.01} min={-90} max={90} />
            <NumberField label="Longitude" value={draft.location.longitudeDeg}
              onChange={(v) => patch('location', { longitudeDeg: v })} suffix="°" step={0.01} min={-180} max={180} />
          </div>
          {showMap && (
            <>
              <LocationPicker latitudeDeg={draft.location.latitudeDeg}
                longitudeDeg={draft.location.longitudeDeg}
                onPick={(latitudeDeg, longitudeDeg) =>
                  patch('location', { latitudeDeg, longitudeDeg })} />
              <p className="text-xs text-fgdim">
                Click the map to set the location. Tiles load from openstreetmap.org.
              </p>
            </>
          )}
          <p className="text-xs text-fgdim">Used for the sky overlay and day/night switching.</p>
        </div>
      </Card>

      <Card title="Processing">
        <div className="flex flex-col gap-4">
          <Toggle label="Generate keogram" checked={draft.processing.keogram}
            onChange={(v) => patch('processing', { keogram: v })} />
          <Toggle label="Generate star trails" checked={draft.processing.startrails}
            onChange={(v) => patch('processing', { startrails: v })} />
          <Toggle label="Generate day timelapse" checked={draft.processing.timelapseDay}
            onChange={(v) => patch('processing', { timelapseDay: v })} />
          <Toggle label="Generate night timelapse" checked={draft.processing.timelapseNight}
            onChange={(v) => patch('processing', { timelapseNight: v })} />
          <div className="grid grid-cols-2 gap-3">
            <NumberField label="Timelapse FPS" value={draft.processing.timelapseFps}
              onChange={(v) => patch('processing', { timelapseFps: v })} min={1} max={60} />
            <NumberField label="Star trails brightness limit" value={draft.processing.startrailsBrightnessLimit}
              onChange={(v) => patch('processing', { startrailsBrightnessLimit: v })} min={0} max={255} />
          </div>
          <Input label="Extra ffmpeg args" value={draft.processing.timelapseExtraArgs}
            onChange={(v) => patch('processing', { timelapseExtraArgs: v })} />
        </div>
      </Card>

      <Card title="Storage">
        <div className="grid grid-cols-2 gap-3">
          <NumberField label="Keep frames" value={draft.storage.framesRetentionDays}
            onChange={(v) => patch('storage', { framesRetentionDays: v })} suffix="days" min={1} />
          <NumberField label="Keep keograms/trails/videos" value={draft.storage.artifactsRetentionDays}
            onChange={(v) => patch('storage', { artifactsRetentionDays: v })} suffix="days" min={1} />
        </div>
      </Card>

      <Card title="Security">
        <div className="flex flex-col gap-3">
          <Input label="Current password" type="password" value={oldPw} onChange={setOldPw}
            autoComplete="current-password" />
          <Input label="New password" type="password" value={newPw} onChange={setNewPw}
            autoComplete="new-password" />
          {pwMessage && (
            <p className={`text-sm ${pwMessage.ok ? 'text-ok' : 'text-danger'}`}>{pwMessage.text}</p>
          )}
          <Button variant="ghost" onClick={changePassword} disabled={!oldPw || !newPw}>
            Change password
          </Button>
        </div>
      </Card>
    </div>
  )
}
