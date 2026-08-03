#!/usr/bin/env bash
# Root pre-start hook (ExecStartPre=+ in rskycam.service): installs a
# self-update staged by the (unprivileged) service under
# /var/lib/rskycam/update/.
#
# Invariants:
# - NEVER blocks service start: every path exits 0.
# - NEVER installs bytes whose sha256 it did not itself fetch from
#   GitHub over TLS — a compromised service user cannot use the staging
#   dir to escalate to root with an arbitrary binary.
set -u

REPO="awitwicki/rskycam"
UPDATE_DIR="/var/lib/rskycam/update"
TARBALL="$UPDATE_DIR/rskycam-aarch64.tar.gz"
TAG_FILE="$UPDATE_DIR/tag"
BIN="/usr/local/bin/rskycam"

log() { echo "rskycam-apply-update: $*" >&2; }
discard() {
  log "$1 — discarding staged update"
  rm -rf "$UPDATE_DIR"
  exit 0
}

[ -f "$TARBALL" ] && [ -f "$TAG_FILE" ] || exit 0

tag=$(cat "$TAG_FILE")
# Strict tag shape: it becomes part of a URL fetched as root, so an
# attacker-controlled tag must not be able to point elsewhere.
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || discard "malformed tag '$tag'"

sha_url="https://github.com/$REPO/releases/download/$tag/rskycam-aarch64.tar.gz.sha256"
sha=$(curl -fsSL --max-time 15 "$sha_url") || discard "cannot fetch checksum (offline?)"
expected=$(echo "$sha" | awk '{print $1}')
actual=$(sha256sum "$TARBALL" | awk '{print $1}')
[ -n "$expected" ] && [ "$expected" = "$actual" ] || discard "checksum mismatch"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$TARBALL" -C "$tmp" rskycam || discard "tar extraction failed"
chmod +x "$tmp/rskycam"
"$tmp/rskycam" --version >/dev/null 2>&1 || discard "staged binary failed --version"

cp "$BIN" "$BIN.old" 2>/dev/null || true
install -m 755 "$tmp/rskycam" "$BIN"
rm -rf "$UPDATE_DIR"
log "installed $("$BIN" --version 2>/dev/null || echo 'rskycam (version unknown)')"
exit 0
