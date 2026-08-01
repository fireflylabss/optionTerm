# AUR packaging (`optionterm`)

Published: https://aur.archlinux.org/packages/optionterm

## Install

```bash
yay -S optionterm
# or
paru -S optionterm
```

## Automatic publish (recommended)

Every **GitHub Release** runs [`.github/workflows/publish-aur.yml`](../../.github/workflows/publish-aur.yml):

1. Bumps `packaging/aur/PKGBUILD` + `.SRCINFO` for the release tag
2. Commits the bump on `master`
3. Pushes the package to the AUR

### One-time setup

Add the AUR SSH **private** key as a repo secret:

1. GitHub → **Settings → Secrets and variables → Actions**
2. New secret name: `AUR_SSH_PRIVATE_KEY`

```bash
gh secret set AUR_SSH_PRIVATE_KEY < ~/.ssh/aur_synara
```

The public key must already be on your AUR account (same key as optionMusic / opsh).

### Day-to-day

```bash
git tag -a v0.2.2 -m "optionTerm 0.2.2"
git push origin v0.2.2
# → release.yml builds .deb / AppImage
# → publish-aur.yml updates AUR (on release published)
```

Manual re-run: **Actions → Publish AUR → Run workflow**.

## Local publish (fallback)

```bash
./packaging/aur/publish.sh           # push current packaging/
./packaging/aur/publish.sh 0.2.2     # bump + push
```

Uses `~/aur/optionterm` and `~/.ssh/aur_synara` (override with `AUR_SSH_KEY=` / `AUR_DIR=`).
