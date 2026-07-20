import { Map as MapIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { getApi } from '../api/client'
import type { Settings } from '../api/types'
import LocationPicker from '../components/LocationPicker'
import { Button, Card, Input, NumberField, Toggle } from '../components/ui'
import { useStatus } from '../hooks/useStatus'

export default function SettingsPage() {
  const { status } = useStatus()
  const [draft, setDraft] = useState<Settings | null>(null)
  const [showMap, setShowMap] = useState(false)
  const [saved, setSaved] = useState(false)
  const [oldPw, setOldPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [pwMessage, setPwMessage] = useState<{ ok: boolean; text: string } | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    void getApi().getSettings().then(setDraft).catch((e: unknown) => setError(String(e)))
  }, [])

  if (error) return <p className="text-danger">{error}</p>
  if (!draft) return <p className="text-fgdim">Loading…</p>

  const patch = <K extends keyof Settings>(key: K, value: Partial<Settings[K]>) =>
    setDraft((d) => d && { ...d, [key]: { ...d[key], ...value } })

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
      <header className="flex items-center justify-between">
        <h1 className="text-lg font-medium">Settings</h1>
        <div className="flex items-center gap-3">
          {saved && <span className="text-sm text-ok">Saved ✓</span>}
          <Button onClick={save}>Save settings</Button>
        </div>
      </header>

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
            <NumberField label="Capture interval" value={cam.intervalSec}
              onChange={(v) => patch('camera', { intervalSec: v })} suffix="s" min={1} />
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
          <Toggle label="Generate timelapse video" checked={draft.processing.timelapse}
            onChange={(v) => patch('processing', { timelapse: v })} />
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
