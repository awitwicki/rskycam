#!/usr/bin/env bash
# Dev deploy: build rskycam (glibc + embedded UI) and swap the binary on a
# Pi that was set up by installer/install.sh (production layout:
# /usr/local/bin/rskycam, rskycam.service, /var/lib/rskycam).
# Unit/udev/user changes are NOT deployed here — re-run install.sh for those.
# gnu (not musl): the vendored ZWO ASI SDK (assets/asi/libASICamera2.so) is
# a glibc shared object, dlopen-ed at runtime, and the Pi runs Debian/glibc.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${1:-pi@rpiwhite.local}"

(cd frontend && npm run build)
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --features embed-ui

scp target/aarch64-unknown-linux-gnu/release/rskycam "$HOST:rskycam.new"
ssh "$HOST" 'bash -s' <<'REMOTE'
set -eu
sudo systemctl stop rskycam
sudo install -m 755 ~/rskycam.new /usr/local/bin/rskycam
rm ~/rskycam.new
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
echo "rskycam active."
REMOTE
echo "-> http://rpiwhite.local:8080"
