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
DATA_DIR="${RSKYCAM_DATA:-/var/lib/rskycam}"
UPDATE_DIR="$DATA_DIR/update"
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
sha=$(curl -fsSL --proto '=https' --proto-redir '=https' --max-time 15 "$sha_url") || discard "cannot fetch checksum (offline?)"
expected=$(echo "$sha" | awk '{print $1}')

# Copy the staged tarball into our root-owned tmp dir and never read
# $TARBALL again after this single copy: $UPDATE_DIR is writable by the
# unprivileged rskycam service user, so hashing and extracting from
# $TARBALL as two separate reads would leave a TOCTOU window where a
# compromised service user swaps the file between the checksum check and
# the extraction. Hashing and extracting the same already-copied,
# root-owned file closes that window.
#
# Placed under $DATA_DIR rather than the system /tmp: this hook runs
# outside the systemd sandbox (the "+" prefix), so PrivateTmp= does not
# apply to it, and a hardened host could mount /tmp noexec — $DATA_DIR
# is guaranteed exec-able since the app itself runs its binary from
# alongside it on the same filesystem.
tmp=$(mktemp -d -p "$DATA_DIR")
trap 'rm -rf "$tmp"' EXIT
cp "$TARBALL" "$tmp/rskycam-aarch64.tar.gz" || discard "cannot copy staged tarball"
actual=$(sha256sum "$tmp/rskycam-aarch64.tar.gz" | awk '{print $1}')
[ -n "$expected" ] && [ "$expected" = "$actual" ] || discard "checksum mismatch"

tar -xzf "$tmp/rskycam-aarch64.tar.gz" -C "$tmp" rskycam || discard "tar extraction failed"
chmod +x "$tmp/rskycam"
"$tmp/rskycam" --version >/dev/null 2>&1 || discard "staged binary failed --version"

cp "$BIN" "$BIN.old" 2>/dev/null || true
if ! install -m 755 "$tmp/rskycam" "$BIN"; then
  log "installing new binary failed — leaving staged update in place for retry"
  exit 0
fi
rm -rf "$UPDATE_DIR"
log "installed $("$BIN" --version 2>/dev/null || echo 'rskycam (version unknown)')"
exit 0
