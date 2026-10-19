#!/bin/bash
set -euo pipefail

# Runs inside the Arch honeypot container as user mole. Installs a fixture package as a
# dependency so pacman reports it as an orphan, then checks the report lists it and removes nothing.
binary=/usr/local/bin/mole-linux
fixture=mole-fixture-orphan
work=$(mktemp -d)

cat > "$work/PKGBUILD" <<'EOF'
pkgname=mole-fixture-orphan
pkgver=1.0
pkgrel=1
pkgdesc='mole-linux honeypot orphan fixture'
arch=('x86_64')
license=('GPL-3.0-only')
package() {
    mkdir -p "$pkgdir/usr/share/mole-fixture-orphan"
    printf 'fixture\n' > "$pkgdir/usr/share/mole-fixture-orphan/README"
}
EOF
(cd "$work" && makepkg -f >/dev/null 2>&1)
sudo pacman -U --noconfirm --asdeps "$work/$fixture-1.0-1-x86_64.pkg.tar.zst" >/dev/null

orphans=$(pacman -Qdtq 2>/dev/null || true)
if ! grep -qx "$fixture" <<< "$orphans"; then
    printf 'fixture is not reported by pacman -Qdtq:\n%s\n' "$orphans" >&2
    exit 1
fi

preview=$("$binary" clean --dry-run --select orphan-packages)
printf '%s\n' "$preview"
grep -Fq "$fixture" <<< "$preview"
grep -Fq 'report only, nothing removed' <<< "$preview"
grep -Fq 'attention: 1' <<< "$preview"
pacman -Q "$fixture" >/dev/null

output=$("$binary" clean --yes --select orphan-packages)
printf '%s\n' "$output"
grep -Fq 'attention: 1' <<< "$output"
grep -Fq 'applied: 0' <<< "$output"
if grep -Fq 'Failed' <<< "$output"; then
    printf 'orphan report attempted a failing action\n' >&2
    exit 1
fi
pacman -Q "$fixture" >/dev/null || {
    printf 'orphan report removed the fixture package\n' >&2
    exit 1
}
