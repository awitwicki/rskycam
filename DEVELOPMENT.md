# Development

Technical notes for working on rskycam itself. For installing and using
it, see [README.md](README.md).

## Repository layout

- `src/` — Rust backend (capture loop, camera drivers, overlay geometry,
  sensors, system stats, web/API layer).
- `frontend/` — React + TypeScript + Vite SPA. See `frontend/README.md`
  for frontend-specific notes.
- `scripts/deploy-pi.sh` — cross-compile + deploy to a Raspberry Pi over
  SSH, for iterating on a Pi set up with the installer.
- `installer/` — the install script, systemd unit, and udev rule shipped
  in releases.

## Updating / uninstalling an install

**Update:** re-run the install command from the README. It replaces the
binary, udev rule and service, restarts, and never touches
`/var/lib/rskycam` (config + images). Pin a version with
`sudo RSKYCAM_VERSION=v0.5.0.12 bash`.

**Logs:** `journalctl -u rskycam -f` (journald handles rotation).

## Self-update

The sidebar's Update pill (next to the GitHub link) performs a staged
self-update:

1. The service (unprivileged) downloads the newest release tarball and
   its sha256 into `/var/lib/rskycam/update/`, verifies the checksum,
   writes the release tag, replies 202 and exits.
2. systemd (`Restart=always`) restarts the unit.
   `ExecStartPre=+/usr/local/bin/rskycam-apply-update` runs as root: it
   re-fetches the release checksum from GitHub itself, re-verifies the
   staged tarball, smoke-tests the binary (`--version`), installs it to
   `/usr/local/bin/rskycam` (previous kept as `rskycam.old`) and removes
   the staging dir. Every failure discards the staged files and never
   blocks service start.

Rollback after a bad update:

    sudo mv /usr/local/bin/rskycam.old /usr/local/bin/rskycam
    sudo systemctl restart rskycam

Version scheme: CI builds embed `X.Y.Z.<run_number>` (`RSKYCAM_BUILD`
set by the release workflow at compile time); local builds report
`X.Y.Z-dev`. `rskycam --version` prints it; `/api/status` carries it to
the UI.

**Uninstall:**

```bash
sudo systemctl disable --now rskycam
sudo rm /etc/systemd/system/rskycam.service /etc/udev/rules.d/99-asi.rules /usr/local/bin/rskycam
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo rm -r /usr/local/share/doc/rskycam
sudo rm -r /var/lib/rskycam   # deletes all captured images and config
sudo userdel rskycam
```

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

Backend tests: `cargo test` — no hardware required, everything
hardware-facing has a mock/fake, including a fake `ffmpeg`.

## Night artifacts

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

Note: `timelapseExtraArgs` (Settings → Processing) is passed verbatim to
`ffmpeg` — it is an admin-level knob by design; only the authenticated
admin can set it, and it runs with the service's (unprivileged) rights.

## Dev deploys to a Raspberry Pi

For iterating on a Pi that was set up with the installer,
`scripts/deploy-pi.sh` builds the frontend, cross-compiles an
`aarch64-unknown-linux-gnu.2.36` binary with the UI embedded
(`--features embed-ui`), swaps `/usr/local/bin/rskycam` over SSH and
restarts the `rskycam` systemd service. The target is glibc, not musl,
because the vendored ZWO ASI SDK (`assets/asi/libASICamera2.so`) is a
glibc shared object dlopen-ed at runtime.

One-time toolchain setup on the build machine (macOS):

```bash
rustup target add aarch64-unknown-linux-gnu
brew install zig            # cargo-zigbuild uses zig as the cross linker
cargo install cargo-zigbuild
```

Deploy (defaults to `pi@rpiwhite.local`, override with an argument):

```bash
./scripts/deploy-pi.sh
./scripts/deploy-pi.sh pi@other-host.local
```

Changes to the systemd unit or udev rule are not covered — re-run the
installer for those.

## Security model

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
