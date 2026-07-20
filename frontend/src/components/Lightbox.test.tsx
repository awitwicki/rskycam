import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import Lightbox from './Lightbox'

const items = [
  { url: 'data:,a', caption: 'first frame' },
  { url: 'data:,b', caption: 'second frame' },
]

beforeEach(() => cleanup())

describe('Lightbox', () => {
  it('shows the current item with caption and counter', () => {
    render(<Lightbox items={items} index={0} onClose={() => {}} onNavigate={() => {}} />)
    expect(screen.getByText('first frame')).toBeInTheDocument()
    expect(screen.getByText('1 / 2')).toBeInTheDocument()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('navigates with the arrow buttons and keyboard', async () => {
    const onNavigate = vi.fn()
    render(<Lightbox items={items} index={0} onClose={() => {}} onNavigate={onNavigate} />)
    expect(screen.queryByLabelText(/previous image/i)).not.toBeInTheDocument()
    await userEvent.click(screen.getByLabelText(/next image/i))
    expect(onNavigate).toHaveBeenCalledWith(1)
    await userEvent.keyboard('{ArrowRight}')
    expect(onNavigate).toHaveBeenCalledTimes(2)
  })

  it('closes on Escape and on close button', async () => {
    const onClose = vi.fn()
    render(<Lightbox items={items} index={1} onClose={onClose} />)
    await userEvent.keyboard('{Escape}')
    await userEvent.click(screen.getByLabelText(/close/i))
    expect(onClose).toHaveBeenCalledTimes(2)
  })
})
