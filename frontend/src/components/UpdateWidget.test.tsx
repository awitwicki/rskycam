import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import UpdateWidget from './UpdateWidget'
import { setApi } from '../api/client'
import { MockApi } from '../api/mock/mockApi'
import { HttpError } from '../api/realApi'
import { _resetUpdateInfoForTests } from '../hooks/useUpdateInfo'
import type { Status, UpdateInfo } from '../api/types'

function apiWith(update: UpdateInfo, versions: string[]) {
  const api = new MockApi()
  api.getUpdate = async () => update
  const seq = [...versions]
  const origStatus = api.getStatus.bind(api)
  api.getStatus = async (): Promise<Status> => {
    const s = await origStatus()
    return { ...s, version: seq.length > 1 ? seq.shift()! : seq[0] }
  }
  api.applyUpdate = vi.fn(async () => {})
  return api
}

beforeEach(() => _resetUpdateInfoForTests())
afterEach(() => vi.useRealTimers())

describe('UpdateWidget', () => {
  it('shows the plain version when no update is available', async () => {
    setApi(apiWith(
      { current: '0.5.0.7', latest: 'v0.5.0.7', updateAvailable: false, error: null },
      ['0.5.0.7'],
    ))
    render(<UpdateWidget />)
    expect(await screen.findByText('v0.5.0.7')).toBeInTheDocument()
  })

  it('runs the update flow through to the success state', async () => {
    const api = apiWith(
      { current: '0.5.0.7', latest: 'v0.5.0.9', updateAvailable: true, error: null },
      ['0.5.0.7', '0.5.0.9'],
    )
    setApi(api)
    const user = userEvent.setup()
    render(<UpdateWidget />)

    // Open the confirm dialog with real timers running: the hook's one-shot
    // fetch resolves via a React-scheduled update, and enabling fake timers
    // before that update lands makes it hang indefinitely (verified: with
    // vi.useFakeTimers() active from the start, even the initial
    // findByText for the pill never resolves).
    await user.click(await screen.findByText(/Update → v0.5.0.9/))

    // Now switch to fake timers so the 3s poll loop doesn't take 2 real
    // minutes of wall-clock time.
    vi.useFakeTimers()

    // fireEvent, not userEvent, for this click: userEvent.click() wraps the
    // interaction in an async act() that appears to wait on a scheduler
    // macrotask fake timers never fire (confirmed: it hangs even with
    // `userEvent.setup({ advanceTimers: vi.advanceTimersByTime })`).
    // fireEvent.click() dispatches synchronously and doesn't have this problem.
    fireEvent.click(screen.getByRole('button', { name: 'Update' }))
    expect(api.applyUpdate).toHaveBeenCalledOnce()

    // Two poll ticks: first still 0.5.0.7, second returns 0.5.0.9.
    await vi.advanceTimersByTimeAsync(3100)
    await vi.advanceTimersByTimeAsync(3100)

    // vi.waitFor, not testing-library's waitFor: the latter polls via a
    // real setInterval fallback, which is also frozen under fake timers.
    await vi.waitFor(() => expect(screen.getByText(/Updated to v0.5.0.9/)).toBeInTheDocument())
  })

  it('shows the server-rejected message immediately on an HTTP-level apply failure', async () => {
    const api = apiWith(
      { current: '0.5.0.7', latest: 'v0.5.0.9', updateAvailable: true, error: null },
      ['0.5.0.7'],
    )
    // A real HTTP rejection (e.g. 409 "no newer release known", or 503 if
    // the self-update hook isn't installed) — the server never restarted,
    // so this path must not poll and must not wait out the 2-minute
    // timeout: no fake-timer advancement needed here at all.
    api.applyUpdate = vi.fn(async () => {
      throw new HttpError(409, 'no newer release known')
    })
    setApi(api)
    const user = userEvent.setup()
    render(<UpdateWidget />)

    await user.click(await screen.findByText(/Update → v0.5.0.9/))
    await user.click(screen.getByRole('button', { name: 'Update' }))

    expect(api.applyUpdate).toHaveBeenCalledOnce()
    expect(await screen.findByText('no newer release known')).toBeInTheDocument()
  })
})
