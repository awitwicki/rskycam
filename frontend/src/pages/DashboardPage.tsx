import { Maximize2, Moon, Sun } from 'lucide-react'
import { useEffect, useState } from 'react'
import { getApi } from '../api/client'
import type { LightgraphData, OverlayGeometry } from '../api/types'
import Lightbox from '../components/Lightbox'
import Lightgraph from '../components/Lightgraph'
import MoonPhaseIcon from '../components/MoonPhaseIcon'
import OverlayCanvas from '../components/OverlayCanvas'
import { Card, Toggle } from '../components/ui'
import { useStatus } from '../hooks/useStatus'
import { formatExposure, formatGain, formatUptime } from '../lib/format'

type Tone = 'default' | 'ok' | 'warn' | 'danger'
const toneText: Record<Tone, string> = {
  default: 'text-fg', ok: 'text-ok', warn: 'text-warn', danger: 'text-danger',
}
const toneBar: Record<Tone, string> = {
  default: 'bg-accent', ok: 'bg-ok', warn: 'bg-warn', danger: 'bg-danger',
}

/** Compact label/value row with an optional thin meter. */
function SysRow({ label, value, tone = 'default', pct }: {
  label: string
  value: string
  tone?: Tone
  pct?: number
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-xs text-fgdim">{label}</span>
        <span className={`truncate font-mono text-xs ${toneText[tone]}`}>{value}</span>
      </div>
      {pct !== undefined && (
        <div className="mt-1 h-1 rounded bg-panel2" role="meter" aria-label={label}
          aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
          <div className={`h-full rounded ${toneBar[tone]}`}
            style={{ width: `${Math.min(100, pct)}%` }} />
        </div>
      )}
    </div>
  )
}

export default function DashboardPage() {
  const { status, frame } = useStatus()
  const [showOverlay, setShowOverlay] = useState(true)
  const [geometry, setGeometry] = useState<OverlayGeometry | null>(null)
  const [lightgraph, setLightgraph] = useState<LightgraphData | null>(null)
  const [expanded, setExpanded] = useState(false)

  useEffect(() => {
    if (!showOverlay) return
    let live = true
    void getApi().getOverlay({ time: frame?.meta.timestamp }).then((g) => {
      if (live) setGeometry(g)
    })
    return () => { live = false }
  }, [showOverlay, frame?.meta.timestamp])

  useEffect(() => {
    // missing lightgraph only hides the bar — not worth an error banner
    void getApi().getLightgraph().then(setLightgraph).catch(() => setLightgraph(null))
  }, [])

  const sys = status?.system
  const sensor = status?.sensor
  const astro = status?.astro

  return (
    <div className="grid gap-4 lg:grid-cols-3">
      <Card title="Live view" className="lg:col-span-2"
        action={
          <div className="flex items-center gap-3">
            <Toggle label="Overlay" checked={showOverlay} onChange={setShowOverlay} />
            <button aria-label="Fullscreen" onClick={() => setExpanded(true)}
              className="text-fgdim hover:text-fg">
              <Maximize2 size={16} />
            </button>
          </div>
        }>
        <div className="overflow-hidden rounded-lg bg-night">
          {frame ? (
            <button type="button" aria-label="Open live view fullscreen"
              onClick={() => setExpanded(true)}
              className="relative block cursor-zoom-in lg:mx-auto lg:w-fit">
              <img src={frame.url} alt="Latest all-sky frame"
                className="w-full lg:h-auto lg:max-h-[calc(100dvh-14rem)] lg:w-auto lg:max-w-full" />
              {showOverlay && geometry && (
                <OverlayCanvas geometry={geometry} className="absolute inset-0 h-full w-full" />
              )}
            </button>
          ) : (
            <div className="grid aspect-[4/3] place-items-center text-fgdim lg:max-h-[calc(100dvh-14rem)]">
              Waiting for first frame…
            </div>
          )}
        </div>
        {frame && (
          <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 font-mono text-sm text-fgdim">
            <span>{new Date(frame.meta.timestamp).toLocaleTimeString()}</span>
            <span>exp {formatExposure(frame.meta.exposureUs)}</span>
            <span>gain {formatGain(frame.meta.gain)}</span>
            <span>{frame.meta.isNight ? 'night' : 'day'}</span>
          </div>
        )}
      </Card>

      <div className="flex flex-col gap-4">
        <Card title="Capture & system">
          {status?.capture.state === 'capturing' ? (
            <p className="text-ok">● Capturing</p>
          ) : (
            <p className="text-danger">● {status?.capture.message ?? 'Camera unavailable'}</p>
          )}
          {sys && (
            <div className="mt-3 flex flex-col gap-2 border-t border-line pt-3">
              <SysRow label="CPU temp" value={`${sys.cpuTempC.toFixed(0)}°C`}
                tone={sys.cpuTempC > 80 ? 'danger' : sys.cpuTempC > 70 ? 'warn' : 'default'}
                pct={sys.cpuTempC} />
              <SysRow label="RAM"
                value={`${(sys.ramUsedMb / 1024).toFixed(1)} / ${(sys.ramTotalMb / 1024).toFixed(1)} GB`}
                pct={(sys.ramUsedMb / sys.ramTotalMb) * 100} />
              <SysRow label="Disk"
                value={`${sys.diskUsedGb.toFixed(0)} / ${sys.diskTotalGb.toFixed(0)} GB`}
                tone={sys.diskUsedGb / sys.diskTotalGb > 0.9 ? 'danger' : 'default'}
                pct={(sys.diskUsedGb / sys.diskTotalGb) * 100} />
              <SysRow label="Uptime" value={formatUptime(sys.uptimeSec)} />
              <SysRow label="Model" value={sys.model.replace('Raspberry Pi', 'RPi')} />
              <SysRow label="Power"
                value={sys.undervoltageNow ? 'UNDERVOLTAGE' : 'OK'}
                tone={sys.undervoltageNow ? 'danger' : sys.undervoltageSinceBoot ? 'warn' : 'ok'} />
              {!sys.undervoltageNow && sys.undervoltageSinceBoot && (
                <p className="text-[10px] text-warn">undervoltage since boot</p>
              )}
            </div>
          )}
        </Card>
        {astro && (
          <Card title="Sky">
            <div className="flex items-center justify-between gap-3">
              <div className="flex flex-col gap-1.5 font-mono text-sm">
                <span className={astro.sunAltDeg > 0 ? 'text-warn' : 'text-fgdim'}>
                  <Sun size={14} className="mr-1.5 inline" />
                  {astro.sunAltDeg > 0 ? '+' : ''}{astro.sunAltDeg.toFixed(1)}°
                </span>
                <span className={astro.moonAltDeg > 0 ? 'text-fg' : 'text-fgdim'}>
                  <Moon size={14} className="mr-1.5 inline" />
                  {astro.moonAltDeg > 0 ? '+' : ''}{astro.moonAltDeg.toFixed(1)}°
                </span>
              </div>
              <div className="flex items-center gap-2">
                <MoonPhaseIcon pct={astro.moonPhasePct} waxing={astro.moonWaxing} />
                <span className="text-xs leading-tight text-fgdim">
                  {Math.round(astro.moonPhasePct)}%<br />
                  {astro.moonWaxing ? 'waxing' : 'waning'}
                </span>
              </div>
            </div>
            {lightgraph && (
              <div className="mt-3">
                <Lightgraph data={lightgraph} />
              </div>
            )}
          </Card>
        )}
        {sensor && sensor.state !== 'disabled' && (
          <Card title="Weather (BME280)">
            {sensor.reading ? (
              <div className="grid grid-cols-3 gap-2 font-mono">
                <div>
                  <div className="text-2xl">{sensor.reading.temperatureC.toFixed(1)}°C</div>
                  <div className="text-xs text-fgdim">temp</div>
                </div>
                <div>
                  <div className="text-2xl">{Math.round(sensor.reading.pressureHpa)}</div>
                  <div className="text-xs text-fgdim">hPa</div>
                </div>
                {sensor.reading.humidityPct !== undefined && (
                  <div>
                    <div className="text-2xl">{Math.round(sensor.reading.humidityPct)}%</div>
                    <div className="text-xs text-fgdim">hum</div>
                  </div>
                )}
              </div>
            ) : (
              <p className="text-sm text-danger">● Sensor not detected — check the I2C wiring</p>
            )}
          </Card>
        )}
      </div>

      {expanded && frame && (
        <Lightbox
          items={[{
            url: frame.url,
            caption: `${new Date(frame.meta.timestamp).toLocaleString()} · exp ${formatExposure(frame.meta.exposureUs)} · gain ${formatGain(frame.meta.gain)}`,
            downloadName: 'latest.jpg',
          }]}
          index={0}
          onClose={() => setExpanded(false)}
        />
      )}
    </div>
  )
}
