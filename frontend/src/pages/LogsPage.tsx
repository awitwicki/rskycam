import { useEffect, useRef, useState } from 'react'
import { getApi } from '../api/client'
import { Button, Card, Toggle } from '../components/ui'

const LEVELS = ['all', 'info', 'warn', 'error'] as const
type Level = (typeof LEVELS)[number]

/** Match on the padded level token tracing writes ("  INFO rskycam::…"). */
function hasLevel(line: string, level: Exclude<Level, 'all'>): boolean {
  return line.includes(` ${level.toUpperCase()} `)
}

function lineClass(line: string): string {
  if (hasLevel(line, 'error')) return 'text-danger'
  if (hasLevel(line, 'warn')) return 'text-warn'
  return 'text-fg'
}

export default function LogsPage() {
  const [lines, setLines] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [follow, setFollow] = useState(true)
  const [level, setLevel] = useState<Level>('all')
  const [text, setText] = useState('')
  const boxRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    const load = () => {
      getApi().getLogs()
        .then((r) => {
          if (cancelled) return
          setLines(r.lines)
          setError(null)
        })
        .catch((e: unknown) => {
          if (!cancelled) setError(String(e))
        })
    }
    load()
    if (!follow) return () => { cancelled = true }
    const t = setInterval(load, 5000)
    return () => {
      cancelled = true
      clearInterval(t)
    }
  }, [follow])

  // Stick to the newest lines while following.
  useEffect(() => {
    const el = boxRef.current
    if (follow && el) el.scrollTop = el.scrollHeight
  }, [lines, follow])

  const visible = (lines ?? []).filter((l) =>
    (level === 'all' || hasLevel(l, level))
    && (text === '' || l.toLowerCase().includes(text.toLowerCase())))

  return (
    <Card title="Logs">
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <div className="flex gap-1">
          {LEVELS.map((lv) => (
            <Button key={lv} variant={level === lv ? 'primary' : 'ghost'}
              onClick={() => setLevel(lv)} className="!px-2 !py-1 text-xs">
              {lv}
            </Button>
          ))}
        </div>
        <input value={text} onChange={(e) => setText(e.target.value)}
          placeholder="Filter…" aria-label="Filter log lines"
          className="min-w-40 flex-1 rounded-lg border border-line bg-panel2 px-3 py-1.5 text-sm text-fg" />
        <Toggle label="Follow" checked={follow} onChange={setFollow} />
      </div>
      {error && <p className="mb-2 text-sm text-danger">{error}</p>}
      <div ref={boxRef}
        className="max-h-[calc(100dvh-16rem)] overflow-auto rounded-lg bg-night p-3 font-mono text-xs leading-5">
        {visible.map((l, i) => (
          <div key={i} className={`whitespace-pre-wrap break-all ${lineClass(l)}`}>{l}</div>
        ))}
        {lines !== null && visible.length === 0 && (
          <p className="text-fgdim">No matching log lines.</p>
        )}
        {lines === null && !error && <p className="text-fgdim">Loading…</p>}
      </div>
    </Card>
  )
}
