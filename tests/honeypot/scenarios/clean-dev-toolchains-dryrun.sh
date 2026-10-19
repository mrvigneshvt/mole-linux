#!/bin/bash
# Dry run of the Go, mise, Cargo and Gradle rules. Nothing may be cleaned, every fixture
# must remain, no owner clean argv may be recorded, and protected trees must not be listed.
set -euo pipefail

fail() { printf 'clean-dev-toolchains-dryrun: %s\n' "$*" >&2; exit 1; }

bin=$HOME/toolchain-shims
log=$HOME/toolchain-shims.log
mkdir -p "$bin"
: > "$log"

# Fixtures: each owner cache, plus trees the rules must never list.
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

# Shims record their argv (as the real tools would receive it) and answer read-only queries.
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

write_shim go 'case "$*" in
  --version) echo go1.24.0 ;;
  "env GOCACHE") echo "$HOME/.cache/go-build" ;;
esac'
write_shim mise 'case "$*" in
  --version) echo 2026.10.0 ;;
  "cache path") echo "$HOME/.cache/mise" ;;
esac'

trash_before=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
output=$(PATH="$bin:/usr/bin:/bin" CARGO_HOME= /usr/local/bin/mole-linux clean --dry-run --select go,mise,cargo,gradle)
printf '%s\n' "$output"

for reason in 'Go build cache' 'Rust cargo cache' 'Gradle build cache' 'mise cache'; do
    grep -Fq "$reason" <<< "$output" || fail "no preview row for: $reason"
done
grep -Fq "owner \`go clean -cache\`" <<< "$output" || fail "go preview lacks its owner command"
grep -Fq "owner \`mise cache clear\`" <<< "$output" || fail "mise preview lacks its owner command"
grep -Fq "$HOME/.cache/go-build" <<< "$output" || fail "go build cache not named in preview"
grep -Fq "$HOME/.cargo/registry/cache/index.crates.io-mole" <<< "$output" || fail "cargo registry entry not named in preview"
grep -Fq "$HOME/.gradle/caches/build-cache-1/mole" <<< "$output" || fail "gradle build cache entry not named in preview"
grep -Fq "$HOME/.cargo/registry/src" <<< "$output" && fail "cargo registry/src was listed" || true
grep -Fq "$HOME/.cargo/git" <<< "$output" && fail "cargo git store was listed" || true
grep -Fq "$HOME/.gradle/caches/modules-2" <<< "$output" && fail "gradle modules-2 was listed" || true
grep -Fq "$HOME/.gradle/wrapper" <<< "$output" && fail "gradle wrapper was listed" || true
grep -Eq '(^|\s)(go\s+clean|mise\s+cache\s+clear)' "$log" && fail "dry run ran an owner clean" || true
[[ -f "$HOME/.cache/go-build/mole-linux-go-fixture" ]] || fail "dry run removed the go build cache"
[[ -f "$HOME/.cargo/registry/cache/index.crates.io-mole/mole-linux-cargo-fixture" ]] || fail "dry run removed the cargo registry cache"
[[ -f "$HOME/.gradle/caches/build-cache-1/mole/mole-linux-gradle-fixture" ]] || fail "dry run removed the gradle build cache"
trash_after=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
[[ "$trash_after" -eq "$trash_before" ]] || fail "dry run moved something to Trash"
printf 'clean-dev-toolchains-dryrun: ok\n'
