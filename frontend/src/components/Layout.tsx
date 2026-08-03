import { Camera, Compass, Focus, Images, LogOut, ScrollText, Settings } from 'lucide-react'
import { NavLink, Outlet } from 'react-router-dom'
import { useAuth } from '../hooks/useAuth'
import { useUpdateInfo } from '../hooks/useUpdateInfo'
import UpdateWidget from './UpdateWidget'

const REPO_URL = 'https://github.com/awitwicki/rskycam'

const NAV = [
  { to: '/', label: 'Dashboard', icon: Camera },
  { to: '/focus', label: 'Focus', icon: Focus },
  { to: '/nights', label: 'Nights', icon: Images },
  { to: '/overlay', label: 'Overlay', icon: Compass },
  { to: '/logs', label: 'Logs', icon: ScrollText },
  { to: '/settings', label: 'Settings', icon: Settings },
]

function navClass(isActive: boolean, base: string) {
  return `${base} ${isActive ? 'text-accent' : 'text-fgdim hover:text-fg'}`
}

function GithubIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  )
}

export default function Layout() {
  const { logout } = useAuth()
  const update = useUpdateInfo()
  return (
    <div className="min-h-screen md:flex md:h-screen md:overflow-hidden">
      <aside className="hidden border-r border-line bg-panel px-4 py-6 md:flex md:w-52 md:flex-col md:overflow-y-auto">
        <div className="mb-8 font-mono text-lg text-accent">✦ rskycam</div>
        <nav className="flex flex-col gap-1">
          {NAV.map(({ to, label, icon: Icon }) => (
            <NavLink key={to} to={to} end={to === '/'}
              className={({ isActive }) =>
                navClass(isActive, 'flex items-center gap-2 rounded-lg px-3 py-2 text-sm')}>
              <Icon size={16} /> {label}
            </NavLink>
          ))}
        </nav>
        <div className="mt-auto flex flex-col gap-1">
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer"
            className="flex items-center gap-2 px-3 py-2 text-sm text-fgdim hover:text-fg">
            <GithubIcon size={16} /> GitHub
          </a>
          <UpdateWidget />
          <button onClick={logout}
            className="flex items-center gap-2 px-3 py-2 text-sm text-fgdim hover:text-fg">
            <LogOut size={16} /> Log out
          </button>
        </div>
      </aside>

      <header className="flex items-center justify-between border-b border-line bg-panel px-4 py-3 md:hidden">
        <span className="font-mono text-accent">✦ rskycam</span>
        <div className="flex items-center gap-3">
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer" aria-label="GitHub repository"
            className="relative text-fgdim hover:text-fg">
            <GithubIcon size={18} />
            {update?.updateAvailable && (
              <span data-testid="update-dot"
                className="absolute -right-1 -top-1 h-2 w-2 rounded-full bg-accent" />
            )}
          </a>
          <button onClick={logout} aria-label="Log out" className="text-fgdim hover:text-fg">
            <LogOut size={18} />
          </button>
        </div>
      </header>

      <main className="flex-1 p-4 pb-24 md:overflow-y-auto md:p-6 md:pb-6">
        <Outlet />
      </main>

      <nav className="fixed inset-x-0 bottom-0 flex border-t border-line bg-panel md:hidden">
        {NAV.map(({ to, label, icon: Icon }) => (
          <NavLink key={to} to={to} end={to === '/'}
            className={({ isActive }) =>
              navClass(isActive, 'flex flex-1 flex-col items-center gap-0.5 py-2 text-[10px]')}>
            <Icon size={18} /> {label}
          </NavLink>
        ))}
      </nav>
    </div>
  )
}
