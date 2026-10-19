#!/bin/bash
# Apply of the JS owner rules against fake owner tools. Each owner's own cleanup
# command runs once; corepack is absent, so its cache must survive. Canaries are
# checked by the runner after this script exits.
set -euo pipefail

fail() { printf 'clean-dev-js-apply: %s\n' "$*" >&2; exit 1; }

bin=$HOME/js-shims
log=$HOME/js-shims.log
mkdir -p "$bin"
: > "$log"

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
write_shim npm 'case "$*" in
  --version) echo 10.9.0 ;;
  "config get cache") echo "$HOME/.npm" ;;
  "cache clean --force") rm -rf "$HOME/.npm/_cacache" ;;
esac'
write_shim pnpm 'case "$*" in
  --version) echo 11.17.0 ;;
  "store path") echo "$HOME/.local/share/pnpm/store/v11" ;;
  "store prune") rm -f "$HOME/.local/share/pnpm/store/v11/mole-linux-pnpm-fixture" ;;
esac'
# bun fails `pm cache` as on the reference host, so its cache goes to Trash through the sink fallback.
write_shim bun 'case "$*" in
  --version) echo 1.4.2 ;;
  "pm cache") exit 1 ;;
esac'
# corepack is deliberately not installed for this scenario.

# The seeded HOME already has one Trash entry; the bun cache must add exactly one.
trash_before=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
PATH="$bin:/usr/bin:/bin" /usr/local/bin/mole-linux clean --select npm,pnpm,bun,corepack --yes | tee "$HOME/js-apply.out"

# The shim logs tab-separated argv. Clean commands run exactly once each; queries may repeat.
for argv in $'npm\tcache\tclean\t--force' $'pnpm\tstore\tprune'; do
    count=$(grep -Fc "$argv" "$log" || true)
    [[ "$count" -eq 1 ]] || fail "expected exactly one owner call for: $argv (got $count)"
done

[[ ! -e "$HOME/.npm/_cacache/mole-linux-npm-fixture" ]] || fail "npm cache survived its owner clean"
[[ ! -e "$HOME/.local/share/pnpm/store/v11/mole-linux-pnpm-fixture" ]] || fail "pnpm store survived its owner prune"
[[ ! -e "$HOME/.bun/install/cache" ]] || fail "bun cache was not moved out of place"
trash_after=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
[[ $((trash_after - trash_before)) -eq 1 ]] || fail "bun cache was not sent to Trash (Trash went $trash_before -> $trash_after)"
grep -Fq $'bun\tpm\tcache\trm' "$log" && fail "bun ran its rm although its query failed" || true
[[ -f "$HOME/.cache/node/corepack/mole-linux-corepack-fixture" ]] || fail "corepack cache was removed without its owner"
grep -Fq 'Unavailable' "$HOME/js-apply.out" || grep -Fq 'unavailable' "$HOME/js-apply.out" || fail "missing corepack was not reported unavailable"
grep -Fq 'CLEAN applied' "$HOME/.local/state/mole-linux/operations.log" || fail "oplog has no applied CLEAN row"
printf 'clean-dev-js-apply: ok\n'
