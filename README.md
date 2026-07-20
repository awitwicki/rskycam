# rskycam

A single Rust binary (axum web server + capture loop) that serves an
embedded React SPA for running an all-sky camera (imx219/rpicam or ZWO ASI).
Designed to run on a Raspberry Pi with a CSI or ZWO camera, and
optionally a BME280 environmental sensor over I2C.

## Repository layout

- `src/` — Rust backend (capture loop, camera drivers, overlay geometry,
  sensors, system stats, web/API layer).
- `frontend/` — React + TypeScript + Vite SPA. See `frontend/README.md` for
  frontend-specific notes.
- `scripts/deploy-pi.sh` — cross-compile + deploy to a Raspberry Pi over SSH.

## Frontend prototype (no backend needed)

The frontend can run standalone against a mock API (synthetic sky, fake
nights and metrics) — useful for UI work without any camera hardware:

```bash
cd frontend
npm install
npm run dev        # http://localhost:5173, login: admin / pa$$word!0
npm test           # vitest
npm run build      # type-check + production build (writes frontend/dist)
```

## Backend locally with MockCamera

The backend defaults to the `rpicam` camera driver (real imx219/CSI camera
via `rpicam-still`), which isn't available on a dev machine. To run the
backend locally against `MockCamera` instead:

```bash
# 1. Run once with a scratch data dir so config.toml is created
RSKYCAM_DATA=/tmp/rskycam-dev RUST_LOG=rskycam=info cargo run
# Ctrl-C after it logs "listening on http://0.0.0.0:8080"

# 2. Edit /tmp/rskycam-dev/config.toml, under [settings.camera]:
#    driver = "mock"

# 3. Run again — the capture loop now produces synthetic frames
RSKYCAM_DATA=/tmp/rskycam-dev RUST_LOG=rskycam=info cargo run
```

Then hit the API directly, e.g.:

```bash
curl -si -X POST http://localhost:8080/api/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"pa$$word!0"}'
```

By default the server serves the SPA from disk (`frontend/dist`) unless
built with `--features embed-ui` (see below), so run `npm run build` in
`frontend/` first if you want `http://localhost:8080/` to serve the UI too.

Backend tests: `cargo test` (111 tests as of Phase 3, no hardware required —
everything hardware-facing has a mock/fake, including a fake `ffmpeg`).

## Night artifacts (Phase 3)

During the night the backend incrementally maintains `keogram.jpg` (central
pixel column per frame) and `startrails.jpg` (lighten-blend, frames brighter
than the configured mean are skipped) in each night's directory, and at dawn
renders `timelapse.mp4` with ffmpeg (`nice -n 19`, H.264/yuv420p; fps and
extra args configurable in Settings → Processing). Artifacts regenerate on
demand via the Rebuild button on a night's page. Generation errors surface
on the night's page next to the artifact. A background task prunes frames
and whole nights after their configured retention (Settings → Storage).
With "bake overlay into saved frames" enabled, saved frames (and thus the
timelapse) carry the overlay; keogram and star trails always use the clean
pre-overlay pixels. Requires `ffmpeg` in PATH on the Pi.

## Deploying to a Raspberry Pi

`scripts/deploy-pi.sh` builds the frontend, cross-compiles an
`aarch64-unknown-linux-gnu.2.36` binary with the UI embedded
(`--features embed-ui`, via `rust-embed`), and ships it to the Pi over SSH.
The target is glibc, not musl, because the vendored ZWO ASI SDK
(`assets/asi/libASICamera2.so`) is a glibc shared object that gets dlopen-ed
at runtime, and the Pi runs Debian/glibc.

One-time toolchain setup on the build machine (macOS):

```bash
rustup target add aarch64-unknown-linux-gnu
brew install zig            # cargo-zigbuild uses zig as the cross linker
cargo install cargo-zigbuild
```

Deploy (defaults to `pi@rpiwhite.local`, override with an argument):

```bash
./scripts/deploy-pi.sh                    # deploy to pi@rpiwhite.local
./scripts/deploy-pi.sh pi@other-host.local
```

This will:

1. `npm run build` in `frontend/`.
2. `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --features embed-ui`.
3. `scp` the resulting binary to `~/rskycam.new` on the Pi.
4. Over SSH: kill any running `rskycam`, move the new binary into place,
   and restart it in the background (`RSKYCAM_DATA=$HOME/rskycam-data`),
   logging to `~/rskycam.log`.

The Pi process is a plain foreground process under `nohup` — no systemd
unit yet (that, along with a dedicated user and `/var/lib/rskycam`, is
planned for a later phase's install script).

Verify it came up:

```bash
curl -si -X POST http://rpiwhite.local:8080/api/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"pa$$word!0"}'
# → 204 + Set-Cookie
```

Then log in at `http://rpiwhite.local:8080` (admin / `pa$$word!0`).

## Security model (Phase 2)

rskycam's auth is intentionally minimal, sized for a single camera on a
trusted home LAN:

- Single admin user; the password is stored as an argon2id hash.
- A session is a signed, `HttpOnly`, `SameSite=Lax` cookie — there is no
  server-side session store (consistent with the no-database design), so
  sessions are stateless and expire after 7 days.
- Intended for plain HTTP on a trusted LAN — there is no TLS yet.
- A captured session cookie remains valid until it expires; changing the
  password does not revoke other existing sessions.
- There is no login rate limiting; argon2's hashing cost is the only
  throttle on brute-force attempts.

Don't expose this server directly to the internet.
