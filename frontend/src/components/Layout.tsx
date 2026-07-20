import { Camera, Compass, Images, LogOut, Settings } from 'lucide-react'
import { NavLink, Outlet } from 'react-router-dom'
import { useAuth } from '../hooks/useAuth'

const NAV = [
  { to: '/', label: 'Dashboard', icon: Camera },
  { to: '/nights', label: 'Nights', icon: Images },
  { to: '/overlay', label: 'Overlay', icon: Compass },
  { to: '/settings', label: 'Settings', icon: Settings },
]

function navClass(isActive: boolean, base: string) {
  return `${base} ${isActive ? 'text-accent' : 'text-fgdim hover:text-fg'}`
}

export default function Layout() {
  const { logout } = useAuth()
  return (
    <div className="min-h-screen md:flex">
      <aside className="hidden border-r border-line bg-panel px-4 py-6 md:flex md:w-52 md:flex-col">
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
        <button onClick={logout}
          className="mt-auto flex items-center gap-2 px-3 py-2 text-sm text-fgdim hover:text-fg">
          <LogOut size={16} /> Log out
        </button>
      </aside>

      <header className="flex items-center justify-between border-b border-line bg-panel px-4 py-3 md:hidden">
        <span className="font-mono text-accent">✦ rskycam</span>
        <button onClick={logout} aria-label="Log out" className="text-fgdim hover:text-fg">
          <LogOut size={18} />
        </button>
      </header>

      <main className="flex-1 p-4 pb-24 md:p-6 md:pb-6">
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
