#!/bin/bash
# Apply of the Go, mise, Cargo and Gradle rules. Each owner clean runs once, sink-backed
# caches reach Trash, and every protected tree keeps its fixture. Canaries are checked by the
# runner after this script exits.
set -euo pipefail

fail() { printf 'clean-dev-toolchains-apply: %s\n' "$*" >&2; exit 1; }

bin=$HOME/toolchain-shims
log=$HOME/toolchain-shims.log
mkdir -p "$bin"
: > "$log"

mkdir -p "$HOME/.cache/go-build" "$HOME/.cache/mise" \
    "$HOME/.cargo/registry/cache/index.crates.io-mole" "$HOME/.cargo/registry/src/index.crates.io-mole/serde" \
    "$HOME/.cargo/git/db/mole" "$HOME/.cargo/bin" \
    "$HOME/.gradle/caches/build-cache-1/mole" "$HOME/.gradle/caches/modules-2/files-2.1/mole" \
    "$HOME/.gradle/notifications/8.14" "$HOME/.gradle/daemon/8.14" "$HOME/.gradle/workers" \
    "$HOME/.gradle/wrapper/dists/mole"
printf 'mole-linux-go-fixture\n' > "$HOME/.cache/go-build/mole-linux-go-fixture"
printf 'mole-linux-mise-fixture\n' > "$HOME/.cache/mise/mole-linux-mise-fixture"
printf 'mole-linux-cargo-fixture\n' > "$HOME/.cargo/registry/cache/index.crates.io-mole/mole-linux-cargo-fixture"
printf 'mole-linux-src-fixture\n' > "$HOME/.cargo/registry/src/index.crates.io-mole/serde/lib.rs"
printf 'mole-linux-git-fixture\n' > "$HOME/.cargo/git/db/mole/HEAD"
printf 'mole-linux-gradle-fixture\n' > "$HOME/.gradle/caches/build-cache-1/mole/mole-linux-gradle-fixture"
printf 'mole-linux-modules-fixture\n' > "$HOME/.gradle/caches/modules-2/files-2.1/mole/mole-linux-modules-fixture"
printf 'mole-linux-wrapper-fixture\n' > "$HOME/.gradle/wrapper/dists/mole/mole-linux-wrapper-fixture"

write_shim() {
    local name=$1 body=$2
    cat > "$bin/$name" <<SHIM
#!/bin/sh
printf '%s' '$name' >> '$log'
for arg in "\$@"; do printf '\t%s' "\$arg" >> '$log'; done
printf '\n' >> '$log'
$body
SHIM
    chmod 755 "$bin/$name"
}

# Each owner command clears only its own cache, as the real owner does.
write_shim go 'case "$*" in
  --version) echo go1.24.0 ;;
  "env GOCACHE") echo "$HOME/.cache/go-build" ;;
  "clean -cache") rm -rf "$HOME/.cache/go-build" ;;
esac'
write_shim mise 'case "$*" in
  --version) echo 2026.10.0 ;;
  "cache path") echo "$HOME/.cache/mise" ;;
  "cache clear") rm -rf "$HOME/.cache/mise" ;;
esac'

trash_before=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
PATH="$bin:/usr/bin:/bin" CARGO_HOME= /usr/local/bin/mole-linux clean --select go,mise,cargo,gradle --yes | tee "$HOME/toolchain-apply.out"

# Owner clean commands run exactly once each; queries may repeat.
for argv in $'go\tclean\t-cache' $'mise\tcache\tclear'; do
    count=$(grep -Fc "$argv" "$log" || true)
    [[ "$count" -eq 1 ]] || fail "expected exactly one owner call for: $argv (got $count)"
done
grep -Fq 'modcache' "$log" && fail "go module cache was addressed" || true

[[ ! -e "$HOME/.cache/go-build/mole-linux-go-fixture" ]] || fail "go build cache survived its owner clean"
[[ ! -e "$HOME/.cache/mise/mole-linux-mise-fixture" ]] || fail "mise cache survived its owner clear"
[[ ! -e "$HOME/.cargo/registry/cache/index.crates.io-mole" ]] || fail "cargo registry cache entry was not moved out of place"
[[ ! -e "$HOME/.gradle/caches/build-cache-1/mole" ]] || fail "gradle build cache entry was not moved out of place"
trash_after=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
[[ $((trash_after - trash_before)) -ge 2 ]] || fail "cargo and gradle entries were not sent to Trash (Trash went $trash_before -> $trash_after)"
[[ -f "$HOME/.cargo/registry/src/index.crates.io-mole/serde/lib.rs" ]] || fail "cargo registry/src was removed"
[[ -f "$HOME/.cargo/git/db/mole/HEAD" ]] || fail "cargo git store was removed"
[[ -f "$HOME/.gradle/caches/modules-2/files-2.1/mole/mole-linux-modules-fixture" ]] || fail "gradle modules-2 was removed"
[[ -f "$HOME/.gradle/wrapper/dists/mole/mole-linux-wrapper-fixture" ]] || fail "gradle wrapper was removed"
grep -Fq 'CLEAN applied' "$HOME/.local/state/mole-linux/operations.log" || fail "oplog has no applied CLEAN row"
printf 'clean-dev-toolchains-apply: ok\n'
