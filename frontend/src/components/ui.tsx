import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from 'react'
import { useId } from 'react'

export function Card({ title, action, className = '', children }: {
  title?: string; action?: ReactNode; className?: string; children: ReactNode
}) {
  return (
    <section className={`rounded-xl border border-line bg-panel p-4 ${className}`}>
      {(title || action) && (
        <header className="mb-3 flex items-center justify-between gap-2">
          <h2 className="text-xs font-medium uppercase tracking-wider text-fgdim">{title}</h2>
          {action}
        </header>
      )}
      {children}
    </section>
  )
}

type Tone = 'default' | 'ok' | 'warn' | 'danger'
const toneText: Record<Tone, string> = {
  default: 'text-fg', ok: 'text-ok', warn: 'text-warn', danger: 'text-danger',
}
const toneBar: Record<Tone, string> = {
  default: 'bg-accent', ok: 'bg-ok', warn: 'bg-warn', danger: 'bg-danger',
}

export function StatTile({ label, value, sub, meterPct, tone = 'default' }: {
  label: string; value: string; sub?: string; meterPct?: number; tone?: Tone
}) {
  return (
    <div className="rounded-xl border border-line bg-panel p-3">
      <div className="text-[10px] uppercase tracking-wider text-fgdim">{label}</div>
      <div className={`mt-1 truncate font-mono text-lg ${toneText[tone]}`}>{value}</div>
      {sub && <div className="truncate text-xs text-fgdim">{sub}</div>}
      {meterPct !== undefined && (
        <div
          className="mt-2 h-1.5 rounded bg-panel2"
          role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100}
          aria-valuenow={Math.round(meterPct)}
        >
          <div className={`h-full rounded ${toneBar[tone]}`} style={{ width: `${Math.min(100, meterPct)}%` }} />
        </div>
      )}
    </div>
  )
}

export function Button({ variant = 'primary', className = '', ...rest }:
  ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'ghost' | 'danger' }) {
  const styles = {
    primary: 'bg-accent text-night hover:brightness-110',
    danger: 'bg-danger text-night hover:brightness-110',
    ghost: 'border border-line text-fg hover:border-accent',
  }[variant]
  return (
    <button
      className={`inline-flex items-center justify-center gap-1.5 rounded-lg px-4 py-2 text-sm font-medium transition ${styles} ${className}`}
      {...rest}
    />
  )
}

export function Input({ label, value, onChange, type = 'text', ...rest }:
  Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'> & {
    label: string; value: string; onChange: (v: string) => void
  }) {
  const id = useId()
  return (
    <label htmlFor={id} className="flex flex-col gap-1 text-sm">
      <span className="text-fgdim">{label}</span>
      <input
        id={id} type={type} value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-lg border border-line bg-panel2 px-3 py-2 text-fg outline-none focus:border-accent"
        {...rest}
      />
    </label>
  )
}

export function Toggle({ label, checked, onChange }: {
  label: string; checked: boolean; onChange: (v: boolean) => void
}) {
  return (
    <button
      type="button" role="switch" aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex items-center gap-2 text-sm text-fgdim hover:text-fg"
    >
      <span className={`h-5 w-9 rounded-full p-0.5 transition ${checked ? 'bg-accent' : 'bg-panel2'}`}>
        <span className={`block h-4 w-4 rounded-full bg-night transition ${checked ? 'translate-x-4 bg-fg' : ''}`} />
      </span>
      {label}
    </button>
  )
}

export function NumberField({ label, value, onChange, step, min, max, suffix }: {
  label: string; value: number; onChange: (v: number) => void
  step?: number; min?: number; max?: number; suffix?: string
}) {
  const id = useId()
  return (
    <label htmlFor={id} className="flex flex-col gap-1 text-sm">
      <span className="text-fgdim">{label}</span>
      <span className="flex items-center gap-2">
        <input
          id={id} type="number" value={value} step={step} min={min} max={max}
          onChange={(e) => onChange(Number(e.target.value))}
          onBlur={() => {
            let v = value
            if (Number.isNaN(v)) v = min ?? 0
            if (min !== undefined && v < min) v = min
            if (max !== undefined && v > max) v = max
            if (v !== value) onChange(v)
          }}
          className="w-full rounded-lg border border-line bg-panel2 px-3 py-2 font-mono text-fg outline-none focus:border-accent"
        />
        {suffix && <span className="shrink-0 text-xs text-fgdim">{suffix}</span>}
      </span>
    </label>
  )
}
