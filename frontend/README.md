# rskycam frontend

React + TypeScript + Vite SPA for rskycam. In Phase 1 it runs entirely
against `MockApi` (synthetic sky, fake nights and metrics) — no backend needed.

## Development

    npm install
    npm run dev        # http://localhost:5173, login: admin / pa$$word!0
    npm test           # vitest
    npm run build      # type-check + production build

## Architecture notes

- All data flows through the `ApiClient` interface (`src/api/client.ts`).
  `src/api/types.ts` is the contract the Rust backend implements in Phase 2.
- Overlay geometry (grids, labels) is computed server-side (mocked in
  `src/lib/overlayGeometry.ts`) and drawn by `OverlayCanvas`; coordinates are
  source-image pixels (960×960 in the mock).
- Mock settings/password persist in localStorage; session in sessionStorage.
