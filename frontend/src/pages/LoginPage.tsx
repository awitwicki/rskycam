import { useState, type FormEvent } from 'react'
import { Navigate, useNavigate } from 'react-router-dom'
import { Button, Input } from '../components/ui'
import { useAuth } from '../hooks/useAuth'

export default function LoginPage() {
  const { authed, login } = useAuth()
  const nav = useNavigate()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')

  if (authed) return <Navigate to="/" replace />

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (await login(username, password)) nav('/', { replace: true })
    else setError('Wrong username or password')
  }

  return (
    <div className="grid min-h-screen place-items-center p-4">
      <form onSubmit={submit}
        className="flex w-full max-w-sm flex-col gap-4 rounded-xl border border-line bg-panel p-6">
        <div className="text-center font-mono text-2xl text-accent">✦ rskycam</div>
        <Input label="Username" value={username} onChange={setUsername} autoComplete="username" />
        <Input label="Password" type="password" value={password} onChange={setPassword}
          autoComplete="current-password" />
        {error && <p className="text-sm text-danger">{error}</p>}
        <Button type="submit">Log in</Button>
        {import.meta.env.VITE_API_MODE !== 'real' && (
          <p className="text-center text-xs text-fgdim">default: admin / pa$$word!0</p>
        )}
      </form>
    </div>
  )
}
