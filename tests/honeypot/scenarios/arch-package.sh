#!/bin/bash
set -euo pipefail

package_dir="$HOME/arch-package"
cd "$package_dir"
makepkg --printsrcinfo > /tmp/mole-linux.SRCINFO
grep -Fq 'pkgname = mole-linux' /tmp/mole-linux.SRCINFO
grep -Fq 'license = GPL-3.0-only' /tmp/mole-linux.SRCINFO
grep -Fq 'archive/refs/tags/v${pkgver}.tar.gz' PKGBUILD
printf 'PKGBUILD metadata validated by makepkg\n'
