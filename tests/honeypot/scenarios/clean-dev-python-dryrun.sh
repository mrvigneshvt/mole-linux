#!/bin/bash
# Dry run of the Python rules against fake owner tools (uv, conda) and the sink-backed
# PyInstaller and Ruff caches. Nothing may be cleaned, every fixture must remain, and no
# owner cleanup argv may be recorded. pip3 is absent from this image, so it must be Unavailable.
set -euo pipefail

fail() { printf 'clean-dev-python-dryrun: %s\n' "$*" >&2; exit 1; }

bin=$HOME/python-shims
log=$HOME/python-shims.log
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

write_shim uv 'case "$*" in
  --version) echo 0.9.0 ;;
  "cache dir") echo "$HOME/.cache/uv" ;;
esac'
write_shim conda 'case "$*" in
  --version) echo 25.1.0 ;;
  "info --json") printf "{\"pkgs_dirs\":[\"%s\"],\"envs\":[]}\n" "$HOME/.conda/pkgs" ;;
esac'

# The seeded HOME already has one Trash entry; compare against this baseline.
trash_before=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
output=$(PATH="$bin:/usr/bin:/bin" /usr/local/bin/mole-linux clean --dry-run --select uv,conda,pyinstaller,ruff-cache)
printf '%s\n' "$output"

grep -Fq "owner \`uv cache prune\`" <<< "$output" || fail "uv prune was not previewed"
grep -Fq "owner \`conda clean --yes --index-cache --tarballs --logfiles\`" <<< "$output" || fail "conda clean was not previewed"
grep -Fq "$HOME/.local/share/pyinstaller/bincache00_py311" <<< "$output" || fail "PyInstaller bincache was not offered"
grep -Fq "$HOME/.cache/ruff/0.9.1" <<< "$output" || fail "Ruff cache child was not offered"
grep -Eq 'pip3? cache|pip cache' <<< "$output" && fail "pip3 is absent but a pip cache was offered"
grep -Fq 'Unavailable' <<< "$output" || grep -Fq 'unavailable' <<< "$output" || fail "missing pip3 was not reported unavailable"

trash_after=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
[[ "$trash_after" -eq "$trash_before" ]] || fail "dry run moved something to Trash"

if grep -Eq $'\t(clean|prune|purge)(\t|$)' "$log"; then
    cat "$log" >&2
    fail "a dry run recorded an owner cleanup command"
fi

for fixture in \
    .cache/uv/mole-linux-uv-fixture \
    .conda/pkgs/mole-linux-conda-fixture \
    .local/share/pyinstaller/bincache00_py311/mole-linux-pyinstaller-fixture \
    .local/share/pyinstaller/config.json \
    .cache/ruff/0.9.1/mole-linux-ruff-fixture; do
    [[ -f "$HOME/$fixture" ]] || fail "dry run removed fixture $fixture"
done
printf 'clean-dev-python-dryrun: ok\n'
