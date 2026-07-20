# CLAUDE.md

rskycam — all-sky camera for Raspberry Pi: one Rust binary (axum web server +
capture loop) serving an embedded React SPA. Spec:
`docs/superpowers/specs/2026-07-14-rskycam-design.md`. How to run
locally (MockCamera, no hardware): see `README.md`.

## Commands & quality gates

Backend (repo root) — all three must pass before every commit:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt        # rustfmt defaults; run before commit, sample code in plans is often unformatted
```

Frontend (`frontend/`):

```bash
npm test         # vitest — does NOT typecheck
npm run build    # tsc -b && vite build — this is the type gate; must pass before commit
```

No test requires hardware: everything hardware-facing has a mock/fake
(MockCamera, fake `rpicam-still` binary in `tests/fixtures/`, pure parsers
fed with captured `/proc` text).

## Architecture invariants

- **Wire contract:** `frontend/src/api/types.ts` is the source of truth
  (camelCase). Rust API/settings types use `#[serde(rename_all = "camelCase")]`
  and must match it field-for-field. Change both sides in the same task.
- **Astro/overlay math is a 1:1 port:** `frontend/src/lib/astro.ts` +
  `overlayGeometry.ts` ↔ `src/overlay/astro.rs` + `geometry.rs`, verified by
  identical test vectors. Never change one side alone; keep output formatting
  (`format.ts` ↔ `geometry.rs`) byte-identical for WYSIWYG overlay labels.
- **Settings:** TOML at `$RSKYCAM_DATA/config.toml`, atomic write
  (tmp + rename). Every new field needs a `#[serde(default = ...)]` so old
  configs on the Pi migrate without loss. Handlers persist-then-adopt and
  adopt at field granularity (never adopt in-memory state that failed to
  persist; never clobber concurrent writes wholesale).
- **Async discipline:** all CPU/disk/blocking work (image pipeline, argon2,
  file reads for status) goes through `spawn_blocking`. Never hold the
  settings `RwLock` across `.await` or across argon2 calls. The capture loop
  is a supervised tokio task that must survive panics in the camera driver
  *and* the camera factory (JoinError is handled, not propagated).
- **Security:** session = signed HttpOnly SameSite=Lax cookie (key in
  `$RSKYCAM_DATA/secret`, 0600 at creation). Any handler that serves files
  must guard against path traversal (see `resolve_spa_path` / `get_file` in
  `src/web/`). Default login: `admin` / `pa$$word!0` (`src/auth.rs`).
- Night = noon-to-noon, dated by the evening; data layout under
  `$RSKYCAM_DATA/images/<date>/` per spec §3.6.

## Workflow

Superpowers flow: brainstorm → spec (`docs/superpowers/specs/`) → plan
(`docs/superpowers/plans/`) → subagent-driven development. Progress ledger:
`.superpowers/sdd/progress.md` — read it before resuming work; carry-forwards
and deferred items live there. Phases: 1 UI prototype ✅, 2 backend core ✅,
3 processing (keogram/startrails/timelapse, overlay baking, retention),
4 distribution (installer, udev, CI releases).

## Deploy

`scripts/deploy-pi.sh` — builds frontend, cross-compiles
aarch64-unknown-linux-gnu.2.36 via cargo-zigbuild with `--features embed-ui`
(gnu, not musl: the vendored ZWO ASI SDK is a glibc shared object),
installs/restarts the `rskycam` systemd unit on the Pi and verifies it came
up. Target: `pi@rpiwhite.local` (Pi 4B 2GB, imx219 NoIR on CSI, I2C disabled,
data dir `/home/pi/rskycam-data`). Logs: `journalctl -u rskycam`.
