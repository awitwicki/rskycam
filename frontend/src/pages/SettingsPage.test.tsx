import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setApi } from '../api/client'
import { MockApi } from '../api/mock/mockApi'
import SettingsPage from './SettingsPage'

vi.mock('leaflet', () => {
  const map = { setView: () => map, on: () => {}, invalidateSize: () => {}, remove: () => {} }
  return {
    map: () => map,
    tileLayer: () => ({ addTo: () => {} }),
    divIcon: () => ({}),
    marker: () => ({
      addTo() {
        return this
      },
      setLatLng() {},
    }),
  }
})
vi.mock('leaflet/dist/leaflet.css', () => ({}))

let api: MockApi

beforeEach(() => {
  cleanup()
  localStorage.clear()
  sessionStorage.clear()
  api = new MockApi({ renderFrame: () => 'data:,x' })
  setApi(api)
})

describe('SettingsPage', () => {
  it('loads and shows current values', async () => {
    render(<SettingsPage />)
    await waitFor(() => expect(screen.getByLabelText(/latitude/i)).toHaveValue(50.45))
    expect(screen.getByLabelText(/longitude/i)).toHaveValue(30.52)
    expect(screen.getByRole('switch', { name: /auto exposure/i })).toBeChecked()
  })

  it('saves edited values through the api', async () => {
    render(<SettingsPage />)
    const lat = await screen.findByLabelText(/latitude/i)
    await userEvent.clear(lat)
    await userEvent.type(lat, '48.85')
    await userEvent.click(screen.getByRole('button', { name: /^save settings$/i }))
    await screen.findByText(/saved/i)
    expect((await api.getSettings()).location.latitudeDeg).toBe(48.85)
  })

  it('saves the capture resolution preset through the api', async () => {
    render(<SettingsPage />)
    const res = await screen.findByLabelText(/resolution/i)
    await userEvent.selectOptions(res, '800x600')
    await userEvent.click(screen.getByRole('button', { name: /^save settings$/i }))
    await screen.findByText(/saved/i)
    const s = await api.getSettings()
    expect(s.camera.captureWidth).toBe(800)
    expect(s.camera.captureHeight).toBe(600)
  })

  it('offers only resolutions the camera supports', async () => {
    render(<SettingsPage />)
    const res = (await screen.findByLabelText(/resolution/i)) as HTMLSelectElement
    const values = Array.from(res.options).map((o) => o.value)
    expect(values).toContain('1280x960') // native mock max
    expect(values).not.toContain('3280x2464') // exceeds the mock sensor, and not current
    // 1640x1232 exceeds the mock's own sensor caps but is the saved 'current' value,
    // so it must stay in the list rather than be silently dropped.
    expect(values).toContain('1640x1232')
  })

  it('opens the OpenStreetMap picker from the Location card', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)
    expect(screen.queryByTestId('location-map')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: /pick on map/i }))
    expect(await screen.findByTestId('location-map')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: /hide map/i }))
    expect(screen.queryByTestId('location-map')).not.toBeInTheDocument()
  })

  it('shows live sensor status and saves the sensor toggle', async () => {
    render(<SettingsPage />)
    const toggle = await screen.findByRole('switch', { name: /bme280/i })
    expect(toggle).toBeChecked()
    await screen.findByText(/● Detected/)
    await userEvent.click(toggle)
    await screen.findByText(/save settings to apply/i)
    await userEvent.click(screen.getByRole('button', { name: /^save settings$/i }))
    await screen.findByText(/saved/i)
    expect((await api.getSettings()).sensor.enabled).toBe(false)
  })

  it('changes the password with correct old password', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)
    await userEvent.type(screen.getByLabelText(/current password/i), 'pa$$word!0')
    await userEvent.type(screen.getByLabelText(/new password/i), 'hunter2')
    await userEvent.click(screen.getByRole('button', { name: /change password/i }))
    expect(await screen.findByText(/password changed/i)).toBeInTheDocument()
    expect(await api.login('admin', 'hunter2')).toBe(true)
  })

  it('rejects a wrong old password', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)
    await userEvent.type(screen.getByLabelText(/current password/i), 'wrong')
    await userEvent.type(screen.getByLabelText(/new password/i), 'hunter2')
    await userEvent.click(screen.getByRole('button', { name: /change password/i }))
    expect(await screen.findByText(/current password is incorrect/i)).toBeInTheDocument()
  })

  it('saves timelapse extra ffmpeg args through the api', async () => {
    render(<SettingsPage />)
    const args = await screen.findByLabelText(/extra ffmpeg args/i)
    await userEvent.type(args, '-preset veryfast')
    await userEvent.click(screen.getByRole('button', { name: /^save settings$/i }))
    await screen.findByText(/saved/i)
    expect((await api.getSettings()).processing.timelapseExtraArgs).toBe('-preset veryfast')
  })

  it('shows the confirm dialog and starts a sweep on confirm', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)
    const spy = vi.spyOn(api, 'startDarksCapture')
    await userEvent.click(screen.getByRole('button', { name: /capture dark sweep/i }))
    expect(screen.getByRole('dialog', { name: /confirm dark sweep/i })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: /^start sweep$/i }))
    expect(spy).toHaveBeenCalled()
  })

  it('disables the sweep button while the camera is unavailable', async () => {
    // The backend rejects a sweep with 503 in this state, so the UI must not
    // offer it in the first place.
    const base = await api.getStatus()
    vi.spyOn(api, 'getStatus').mockResolvedValue({
      ...base,
      capture: { state: 'camera_unavailable', message: 'no camera' },
    })
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /capture dark sweep/i })).toBeDisabled(),
    )
  })

  it('exports the current settings as a downloadable JSON file', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)

    let captured: Blob | null = null
    const url = 'blob:mock-url'
    vi.spyOn(URL, 'createObjectURL').mockImplementation((obj: Blob | MediaSource) => {
      captured = obj as Blob
      return url
    })
    const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})

    await userEvent.click(screen.getByRole('button', { name: /^export$/i }))

    expect(clickSpy).toHaveBeenCalledTimes(1)
    expect(revoke).toHaveBeenCalledWith(url)
    expect(captured).not.toBeNull()
    const text = await (captured as unknown as Blob).text()
    const parsed = JSON.parse(text)
    expect(parsed.location.latitudeDeg).toBe(50.45)
    expect(parsed.camera.driver).toBe('mock')
  })

  it('imports a settings file into the draft, applied only after Save', async () => {
    render(<SettingsPage />)
    const lat = await screen.findByLabelText(/latitude/i)
    expect(lat).toHaveValue(50.45)

    const exported = await api.getSettings()
    const imported = { ...exported, location: { ...exported.location, latitudeDeg: 12.34 } }
    const file = new File([JSON.stringify(imported)], 'rskycam-settings.json', {
      type: 'application/json',
    })
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    await userEvent.upload(input, file)

    await waitFor(() => expect(lat).toHaveValue(12.34))
    // Not yet persisted — only loaded into the draft for review.
    expect((await api.getSettings()).location.latitudeDeg).toBe(50.45)

    await userEvent.click(screen.getByRole('button', { name: /^save settings$/i }))
    await screen.findByText(/saved/i)
    expect((await api.getSettings()).location.latitudeDeg).toBe(12.34)
  })

  it('rejects an import file that is not a valid settings shape', async () => {
    render(<SettingsPage />)
    await screen.findByLabelText(/latitude/i)

    const file = new File([JSON.stringify({ foo: 'bar' })], 'garbage.json', {
      type: 'application/json',
    })
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    await userEvent.upload(input, file)

    expect(await screen.findByText(/import failed/i)).toBeInTheDocument()
    expect((await api.getSettings()).location.latitudeDeg).toBe(50.45) // untouched
  })

  it('lists captured darks and can clear them', async () => {
    await api.startDarksCapture()
    await new Promise((r) => setTimeout(r, 3500)) // let the mock sweep finish
    render(<SettingsPage />)
    // Five exposure stops each get a gain-16 dark, so multiple entries match; wait for them all.
    await screen.findAllByText(/gain 16\.00/i)
    await userEvent.click(screen.getByRole('button', { name: /clear darks/i }))
    await waitFor(() => expect(screen.getByText(/no darks captured yet/i)).toBeInTheDocument())
  }, 10000)
})
