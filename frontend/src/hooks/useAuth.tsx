import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from 'react'
import { getApi } from '../api/client'

interface AuthValue {
  authed: boolean
  login: (username: string, password: string) => Promise<boolean>
  logout: () => void
}

const AuthCtx = createContext<AuthValue | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [authed, setAuthed] = useState(() => getApi().isAuthenticated())

  const login = useCallback(async (username: string, password: string) => {
    const ok = await getApi().login(username, password)
    if (ok) setAuthed(true)
    return ok
  }, [])

  const logout = useCallback(() => {
    void getApi().logout()
    setAuthed(false)
  }, [])

  useEffect(() => {
    const onUnauthorized = () => setAuthed(false)
    window.addEventListener('rskycam:unauthorized', onUnauthorized)
    return () => window.removeEventListener('rskycam:unauthorized', onUnauthorized)
  }, [])

  return <AuthCtx.Provider value={{ authed, login, logout }}>{children}</AuthCtx.Provider>
}

export function useAuth(): AuthValue {
  const v = useContext(AuthCtx)
  if (!v) throw new Error('useAuth must be used inside AuthProvider')
  return v
}
