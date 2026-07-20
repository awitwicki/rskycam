import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'
import { setApi } from './api/client'
import { MockApi } from './api/mock/mockApi'
import App from './App'

beforeEach(() => {
  cleanup()
  localStorage.clear()
  sessionStorage.clear()
  setApi(new MockApi({ renderFrame: () => 'data:image/jpeg;base64,x' }))
  window.history.pushState({}, '', '/')
})

describe('auth flow', () => {
  it('redirects unauthenticated visitors to the login form', () => {
    render(<App />)
    expect(screen.getByLabelText(/username/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/password/i)).toBeInTheDocument()
  })

  it('rejects wrong credentials with an error message', async () => {
    render(<App />)
    await userEvent.type(screen.getByLabelText(/username/i), 'admin')
    await userEvent.type(screen.getByLabelText(/password/i), 'wrong')
    await userEvent.click(screen.getByRole('button', { name: /log in/i }))
    expect(await screen.findByText(/wrong username or password/i)).toBeInTheDocument()
  })

  it('logs in with default credentials and reaches the dashboard', async () => {
    render(<App />)
    await userEvent.type(screen.getByLabelText(/username/i), 'admin')
    await userEvent.type(screen.getByLabelText(/password/i), 'pa$$word!0')
    await userEvent.click(screen.getByRole('button', { name: /log in/i }))
    expect(await screen.findByText(/live view/i)).toBeInTheDocument()
  })
})
