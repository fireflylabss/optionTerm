#!/usr/bin/env bash
# Publish / update optionterm on the AUR (local helper).
# Usage: ./packaging/aur/publish.sh           # push current packaging/
#        ./packaging/aur/publish.sh 0.2.1     # bump + push
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
AUR_DIR="${AUR_DIR:-$HOME/aur/optionterm}"
SSH_KEY="${AUR_SSH_KEY:-$HOME/.ssh/aur_synara}"
REMOTE="ssh://aur@aur.archlinux.org/optionterm.git"

if [[ -n "${1:-}" ]]; then "$ROOT/packaging/aur/bump.sh" "$1"; fi
mkdir -p "$AUR_DIR"
cp "$ROOT/packaging/aur/PKGBUILD" "$AUR_DIR/PKGBUILD"
cp "$ROOT/packaging/aur/.SRCINFO" "$AUR_DIR/.SRCINFO"

if [[ ! -d "$AUR_DIR/.git" ]]; then
  git -C "$AUR_DIR" init -b master
  git -C "$AUR_DIR" remote add origin "$REMOTE"
fi

export GIT_SSH_COMMAND="ssh -i ${SSH_KEY} -o IdentitiesOnly=yes"
git -C "$AUR_DIR" fetch origin master 2>/dev/null || true
git -C "$AUR_DIR" pull --rebase origin master 2>/dev/null || true
git -C "$AUR_DIR" add PKGBUILD .SRCINFO
if ! git -C "$AUR_DIR" diff --cached --quiet; then
  VER="$(sed -n 's/^pkgver=//p' "$AUR_DIR/PKGBUILD")"
  git -C "$AUR_DIR" commit -m "optionterm ${VER}"
fi
git -C "$AUR_DIR" push -u origin HEAD:master
