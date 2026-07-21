#!/usr/bin/env bash
# rskycam installer/updater for Raspberry Pi OS (64-bit).
#
#   curl -fsSL https://raw.githubusercontent.com/awitwicki/rskycam/main/installer/install.sh | sudo bash
#
# Fresh run: installs ffmpeg, the latest rskycam release binary, the ZWO
# udev rule, a dedicated `rskycam` system user with /var/lib/rskycam, and
# a systemd service — then starts it and prints the dashboard URL.
# Re-run: updates binary/unit/udev rule and restarts the service. Never
# touches /var/lib/rskycam (config, images).
#
# Pin a version:  curl ... | sudo RSKYCAM_VERSION=v0.4.0 bash
set -euo pipefail

REPO="awitwicki/rskycam"
ASSET="rskycam-aarch64.tar.gz"
DATA_DIR="/var/lib/rskycam"
BIN="/usr/local/bin/rskycam"
DOC_DIR="/usr/local/share/doc/rskycam"

die() { echo "install.sh: $*" >&2; exit 1; }

main() {
  [ "$(id -u)" -eq 0 ] || die "must run as root: curl ... | sudo bash"
  [ "$(uname -m)" = "aarch64" ] || die "aarch64 only (Raspberry Pi OS 64-bit); this is $(uname -m)"
  command -v systemctl >/dev/null || die "systemd is required"
  command -v apt-get >/dev/null || die "apt-based system required (Raspberry Pi OS / Debian)"
  [ "$(dpkg --print-architecture)" = "arm64" ] || die "64-bit userland (arm64) required; this is $(dpkg --print-architecture)"

  if ! command -v ffmpeg >/dev/null || ! command -v curl >/dev/null; then
    echo "-> installing dependencies (ffmpeg, curl)"
    apt-get update -qq
    apt-get install -y -qq ffmpeg curl ca-certificates
  fi

  if [ -n "${RSKYCAM_VERSION:-}" ]; then
    url="https://github.com/$REPO/releases/download/$RSKYCAM_VERSION/$ASSET"
  else
    url="https://github.com/$REPO/releases/latest/download/$ASSET"
  fi

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  echo "-> downloading $url"
  curl -fsSL -o "$tmp/$ASSET" "$url"
  curl -fsSL -o "$tmp/$ASSET.sha256" "$url.sha256"
  (cd "$tmp" && sha256sum -c "$ASSET.sha256" >/dev/null) || die "checksum mismatch"
  tar -xzf "$tmp/$ASSET" -C "$tmp"

  if ! id rskycam >/dev/null 2>&1; then
    echo "-> creating rskycam system user"
    useradd --system --home-dir "$DATA_DIR" --no-create-home \
      --shell /usr/sbin/nologin rskycam
  fi
  # Camera (rpicam) and I2C sensor access, where those groups exist.
  for grp in video i2c; do
    if getent group "$grp" >/dev/null; then usermod -aG "$grp" rskycam; fi
  done
  install -d -m 750 -o rskycam -g rskycam "$DATA_DIR"

  # Stop before replacing the binary: writing over a running executable
  # fails with ETXTBSY. On a fresh install this is a no-op.
  systemctl stop rskycam 2>/dev/null || true

  install -m 755 "$tmp/rskycam" "$BIN"
  install -d -m 755 "$DOC_DIR"
  install -m 644 "$tmp/LICENSE" "$tmp/ASI-LICENSE" "$DOC_DIR/"
  install -m 644 "$tmp/99-asi.rules" /etc/udev/rules.d/99-asi.rules
  udevadm control --reload-rules || true
  udevadm trigger --attr-match=idVendor=03c3 2>/dev/null || true
  install -m 644 "$tmp/rskycam.service" /etc/systemd/system/rskycam.service

  systemctl daemon-reload
  systemctl enable rskycam >/dev/null
  systemctl restart rskycam

  echo "-> waiting for rskycam to come up"
  for _ in $(seq 1 15); do
    if systemctl is-active --quiet rskycam &&
       journalctl -u rskycam --since "-60s" --no-pager 2>/dev/null | grep -q "listening on"; then
      ip=$(hostname -I 2>/dev/null | awk '{print $1}')
      echo
      echo "rskycam is running (and enabled at boot)."
      echo "  Dashboard:     http://$(hostname).local:8080  (or http://${ip:-<pi-ip>}:8080)"
      # shellcheck disable=SC2016 # literal $$/! — single quotes are deliberate, not a missed expansion
      echo '  Default login: admin / pa$$word!0 — change it in Settings.'
      echo "  Data & config: $DATA_DIR"
      echo "  Logs:          journalctl -u rskycam -f"
      exit 0
    fi
    sleep 1
  done
  echo "rskycam failed to start:" >&2
  journalctl -u rskycam -n 25 --no-pager >&2 || true
  exit 1
}

main "$@"
