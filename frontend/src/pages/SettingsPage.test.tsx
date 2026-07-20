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
})
