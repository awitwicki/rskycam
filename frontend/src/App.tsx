import { BrowserRouter, Navigate, Outlet, Route, Routes } from 'react-router-dom'
import Layout from './components/Layout'
import { AuthProvider, useAuth } from './hooks/useAuth'
import DashboardPage from './pages/DashboardPage'
import LoginPage from './pages/LoginPage'
import NightDetailPage from './pages/NightDetailPage'
import NightsPage from './pages/NightsPage'
import OverlayEditorPage from './pages/OverlayEditorPage'
import SettingsPage from './pages/SettingsPage'

function RequireAuth() {
  const { authed } = useAuth()
  return authed ? <Outlet /> : <Navigate to="/login" replace />
}

export default function App() {
  return (
    <AuthProvider>
      <BrowserRouter>
        <Routes>
          <Route path="/login" element={<LoginPage />} />
          <Route element={<RequireAuth />}>
            <Route element={<Layout />}>
              <Route path="/" element={<DashboardPage />} />
              <Route path="/nights" element={<NightsPage />} />
              <Route path="/nights/:date" element={<NightDetailPage />} />
              <Route path="/overlay" element={<OverlayEditorPage />} />
              <Route path="/settings" element={<SettingsPage />} />
            </Route>
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </BrowserRouter>
    </AuthProvider>
  )
}
