#!/usr/bin/env bash
# Build rskycam for the Pi (glibc + embedded UI) and (re)deploy it as a
# systemd service on the target host, so it survives reboots and crashes.
# gnu (not musl): the vendored ZWO ASI SDK (assets/asi/libASICamera2.so) is a
# glibc shared object, dlopen-ed at runtime, and the Pi runs Debian/glibc.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${1:-pi@rpiwhite.local}"

(cd frontend && npm run build)
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --features embed-ui

scp target/aarch64-unknown-linux-gnu/release/rskycam "$HOST:~/rskycam.new"
scp installer/99-asi.rules "$HOST:~/99-asi.rules.new"
ssh "$HOST" 'bash -s' <<'REMOTE'
set -eu
RUSER=$(id -un)
RHOME=$HOME

# Install/refresh the unit (idempotent — safe to run on every deploy).
sudo tee /etc/systemd/system/rskycam.service >/dev/null <<UNIT
[Unit]
Description=rskycam all-sky camera
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$RUSER
Environment=HOME=$RHOME
Environment=RSKYCAM_DATA=$RHOME/rskycam-data
Environment=RUST_LOG=rskycam=info
WorkingDirectory=$RHOME
ExecStart=$RHOME/rskycam
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT

sudo systemctl daemon-reload

# Install/refresh the udev rule so the ZWO camera is accessible without root
# (idempotent — safe to run on every deploy).
sudo mv ~/99-asi.rules.new /etc/udev/rules.d/99-asi.rules
sudo chown root:root /etc/udev/rules.d/99-asi.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --attr-match=idVendor=03c3 2>/dev/null || true

# Free the port whether the old instance was systemd- or nohup-managed.
sudo systemctl stop rskycam 2>/dev/null || true
pkill -x rskycam 2>/dev/null || true
sleep 1
mv ~/rskycam.new ~/rskycam && chmod +x ~/rskycam
sudo systemctl enable rskycam >/dev/null
sudo systemctl restart rskycam
sleep 3
if ! systemctl is-active --quiet rskycam; then
  echo "rskycam failed to start under systemd:" >&2
  journalctl -u rskycam -n 20 --no-pager >&2
  exit 1
fi
if ! journalctl -u rskycam --since "-30s" --no-pager | grep -q "listening on"; then
  echo "rskycam started but never reported listening:" >&2
  journalctl -u rskycam -n 20 --no-pager >&2
  exit 1
fi
echo "rskycam active and enabled (autostarts at boot)."
REMOTE
echo "→ http://rpiwhite.local:8080"
