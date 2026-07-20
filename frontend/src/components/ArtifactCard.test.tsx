import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import ArtifactCard from './ArtifactCard'

const img = (url: string) => <img src={url} alt="artifact" />

describe('ArtifactCard', () => {
  it('ready: renders children and a download link', () => {
    render(<ArtifactCard title="Keogram" artifact={{ state: 'ready', url: '/k.jpg' }}>{img}</ArtifactCard>)
    expect(screen.getByAltText('artifact')).toHaveAttribute('src', '/k.jpg')
    expect(screen.getByRole('link', { name: /download/i })).toHaveAttribute('href', '/k.jpg')
  })

  it('generating: shows progress text', () => {
    render(<ArtifactCard title="Timelapse" artifact={{ state: 'generating' }}>{img}</ArtifactCard>)
    expect(screen.getByText(/generating/i)).toBeInTheDocument()
  })

  it('error: shows the message', () => {
    render(<ArtifactCard title="Timelapse" artifact={{ state: 'error', message: 'ffmpeg exited with code 1' }}>{img}</ArtifactCard>)
    expect(screen.getByText(/ffmpeg exited with code 1/i)).toBeInTheDocument()
  })

  it('disabled: says so', () => {
    render(<ArtifactCard title="Keogram" artifact={{ state: 'disabled' }}>{img}</ArtifactCard>)
    expect(screen.getByText(/disabled in settings/i)).toBeInTheDocument()
  })
})
