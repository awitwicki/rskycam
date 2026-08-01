import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'
import { setApi } from '../api/client'
import { MockApi } from '../api/mock/mockApi'
import LogsPage from './LogsPage'

beforeEach(() => {
  cleanup()
  localStorage.clear()
  setApi(new MockApi())
})

describe('LogsPage', () => {
  it('renders log lines from the API with level tinting', async () => {
    render(<LogsPage />)
    const err = await screen.findByText(/No space left on device/)
    expect(err.className).toContain('text-danger')
    const warn = screen.getByText(/dark capture failed/)
    expect(warn.className).toContain('text-warn')
    expect(screen.getByText(/rskycam listening/)).toBeInTheDocument()
  })

  it('level and text filters hide non-matching lines', async () => {
    render(<LogsPage />)
    await screen.findByText(/rskycam listening/)

    await userEvent.click(screen.getByRole('button', { name: 'error' }))
    expect(screen.getByText(/No space left on device/)).toBeInTheDocument()
    expect(screen.queryByText(/rskycam listening/)).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: 'all' }))
    await userEvent.type(screen.getByLabelText('Filter log lines'), 'keogram')
    expect(screen.getByText(/keogram updated/)).toBeInTheDocument()
    expect(screen.queryByText(/rskycam listening/)).not.toBeInTheDocument()
  })
})
