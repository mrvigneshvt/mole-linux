#!/bin/bash
# Dry run of the JS owner rules against fake owner tools. Nothing may be cleaned,
# every fixture must remain, and no owner cleanup argv may be recorded.
set -euo pipefail

fail() { printf 'clean-dev-js-dryrun: %s\n' "$*" >&2; exit 1; }

bin=$HOME/js-shims
log=$HOME/js-shims.log
mkdir -p "$bin"
: > "$log"

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

write_shim npm 'case "$*" in
  --version) echo 10.9.0 ;;
  "config get cache") echo "$HOME/.npm" ;;
esac'
write_shim pnpm 'case "$*" in
  --version) echo 11.17.0 ;;
  "store path") echo "$HOME/.local/share/pnpm/store/v11" ;;
esac'
write_shim bun 'case "$*" in
  --version) echo 1.4.2 ;;
  "pm cache") echo "$HOME/.bun/install/cache" ;;
esac'
write_shim corepack 'case "$*" in
  --version) echo 0.34.0 ;;
esac'

output=$(PATH="$bin:/usr/bin:/bin" /usr/local/bin/mole-linux clean --dry-run --select npm,pnpm,bun,corepack)
printf '%s\n' "$output"

selected=$(grep -c 'Selected \[development\]' <<< "$output" || true)
[[ "$selected" -eq 4 ]] || fail "expected 4 selected owner caches, got $selected"
for command in 'npm cache clean --force' 'pnpm store prune' 'bun pm cache rm' 'corepack cache clean'; do
    grep -Fq "owner \`$command\`" <<< "$output" || fail "missing owner command in preview: $command"
done

if grep -Eq $'\t(clean|prune|rm)(\t|$)' "$log"; then
    cat "$log" >&2
    fail "a dry run recorded an owner cleanup command"
fi

for fixture in \
    .npm/_cacache/mole-linux-npm-fixture \
    .local/share/pnpm/store/v11/mole-linux-pnpm-fixture \
    .bun/install/cache/mole-linux-bun-fixture \
    .cache/node/corepack/mole-linux-corepack-fixture; do
    [[ -f "$HOME/$fixture" ]] || fail "dry run removed fixture $fixture"
done
printf 'clean-dev-js-dryrun: ok\n'
