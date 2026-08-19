import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { setApi, type ApiClient } from '../api/client'
import { MockApi } from '../api/mock/mockApi'
import type { Status } from '../api/types'
import DashboardPage from './DashboardPage'

beforeEach(() => {
  cleanup()
  localStorage.clear()
  sessionStorage.clear()
})

describe('DashboardPage', () => {
  it('renders live frame, weather and system tiles from MockApi', async () => {
    setApi(new MockApi({ renderFrame: () => 'data:image/jpeg;base64,x' }))
    render(<DashboardPage />)
    await waitFor(() => expect(screen.getByAltText(/latest all-sky frame/i)).toBeInTheDocument())
    expect(screen.getByText(/weather/i)).toBeInTheDocument()
    expect(screen.getByText(/cpu temp/i)).toBeInTheDocument()
    expect(screen.getByText(/ram/i)).toBeInTheDocument()
    expect(screen.getByText(/uptime/i)).toBeInTheDocument()
    expect(screen.getByText(/undervoltage since boot/i)).toBeInTheDocument()
  })

  it('hides the weather card and shows camera error when appropriate', async () => {
    setApi(apiWith({
      capture: { state: 'camera_unavailable', message: 'ASI camera not found' },
      sensor: { state: 'disabled', reading: null },
    }))
    render(<DashboardPage />)
    await waitFor(() => expect(screen.getByText(/asi camera not found/i)).toBeInTheDocument())
    expect(screen.queryByText(/weather/i)).not.toBeInTheDocument()
  })

  it('shows a non-alarming focusing indicator during a focus session', async () => {
    setApi(apiWith({
      capture: { state: 'focusing' },
    }))
    render(<DashboardPage />)
    await waitFor(() => expect(screen.getByText(/focusing/i)).toBeInTheDocument())
    expect(screen.queryByText(/camera unavailable/i)).not.toBeInTheDocument()
    expect(screen.getByText(/focusing/i)).not.toHaveClass('text-danger')
  })

  it('warns when the sensor is enabled but not detected', async () => {
    setApi(apiWith({ sensor: { state: 'not_detected', reading: null } }))
    render(<DashboardPage />)
    await waitFor(() => expect(screen.getByText(/sensor not detected/i)).toBeInTheDocument())
    expect(screen.getByText(/weather/i)).toBeInTheDocument()
  })
})

/** MockApi with a status override; subscribe is silenced so the patch sticks. */
function apiWith(patchStatus: Partial<Status>): ApiClient {
  const base = new MockApi({ renderFrame: () => 'data:,x' })
  return {
    login: base.login.bind(base),
    logout: base.logout.bind(base),
    isAuthenticated: () => true,
    getStatus: async (): Promise<Status> => ({ ...(await base.getStatus()), ...patchStatus }),
    getLightgraph: base.getLightgraph.bind(base),
    getLogs: base.getLogs.bind(base),
    latestImageUrl: () => 'data:,x',
    subscribe: () => () => {},
    getOverlay: base.getOverlay.bind(base),
    getSettings: base.getSettings.bind(base),
    putSettings: base.putSettings.bind(base),
    changePassword: base.changePassword.bind(base),
    getUpdate: base.getUpdate.bind(base),
    applyUpdate: base.applyUpdate.bind(base),
    getNights: base.getNights.bind(base),
    getNight: base.getNight.bind(base),
    rebuildNight: base.rebuildNight.bind(base),
    deleteNight: base.deleteNight.bind(base),
    detectPole: base.detectPole.bind(base),
    startDarksCapture: base.startDarksCapture.bind(base),
    getDarksLibrary: base.getDarksLibrary.bind(base),
    clearDarks: base.clearDarks.bind(base),
    setFocus: base.setFocus.bind(base),
    focusImageUrl: base.focusImageUrl.bind(base),
    focusStarUrl: base.focusStarUrl.bind(base),
  }
}
