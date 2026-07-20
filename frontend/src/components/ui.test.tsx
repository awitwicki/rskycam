import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NumberField } from './ui'

beforeEach(cleanup)

describe('NumberField', () => {
  it('clamps to min/max on blur and recovers from empty input', async () => {
    const onChange = vi.fn()
    render(<NumberField label="Gain" value={200} min={0} max={100} onChange={onChange} />)
    const input = screen.getByLabelText(/gain/i)
    await userEvent.click(input)
    await userEvent.tab() // blur with out-of-range value
    expect(onChange).toHaveBeenLastCalledWith(100)

    onChange.mockClear()
    render(<NumberField label="Min" value={Number.NaN} min={5} onChange={onChange} />)
    const second = screen.getByLabelText(/^min$/i)
    await userEvent.click(second)
    await userEvent.tab()
    expect(onChange).toHaveBeenLastCalledWith(5)
  })
})
