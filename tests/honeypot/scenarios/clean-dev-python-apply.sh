#!/bin/bash
# Apply of the Python rules against fake owner tools. Each owner's own cleanup command runs
# once and removes only its own fixture. PyInstaller's bincache and the Ruff cache child go to
# Trash through the sink. PyInstaller's state file and pip3 (absent) must survive or stay unavailable.
set -euo pipefail

fail() { printf 'clean-dev-python-apply: %s\n' "$*" >&2; exit 1; }

bin=$HOME/python-shims
log=$HOME/python-shims.log
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

# Each owner command removes only its own fixture, as the real owner clears only its own cache.
write_shim uv 'case "$*" in
  --version) echo 0.9.0 ;;
  "cache dir") echo "$HOME/.cache/uv" ;;
  "cache prune") rm -f "$HOME/.cache/uv/mole-linux-uv-fixture" ;;
esac'
write_shim conda 'case "$*" in
  --version) echo 25.1.0 ;;
  "info --json") printf "{\"pkgs_dirs\":[\"%s\"],\"envs\":[]}\n" "$HOME/.conda/pkgs" ;;
  "clean --yes --index-cache --tarballs --logfiles") rm -f "$HOME/.conda/pkgs/mole-linux-conda-fixture" ;;
esac'
# pip3 is deliberately not installed for this scenario.

# The seeded HOME already has one Trash entry; the bincache and the Ruff child add exactly two.
trash_before=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
PATH="$bin:/usr/bin:/bin" /usr/local/bin/mole-linux clean --select uv,conda,pyinstaller,ruff-cache,pip --yes | tee "$HOME/python-apply.out"

# The shim logs tab-separated argv. Owner cleanup commands run exactly once; queries may repeat.
for argv in $'uv\tcache\tprune' $'conda\tclean\t--yes\t--index-cache\t--tarballs\t--logfiles'; do
    count=$(grep -Fc "$argv" "$log" || true)
    [[ "$count" -eq 1 ]] || fail "expected exactly one owner call for: $argv (got $count)"
done

[[ ! -e "$HOME/.cache/uv/mole-linux-uv-fixture" ]] || fail "uv fixture survived its owner prune"
[[ ! -e "$HOME/.conda/pkgs/mole-linux-conda-fixture" ]] || fail "conda fixture survived its owner clean"
[[ ! -e "$HOME/.local/share/pyinstaller/bincache00_py311" ]] || fail "PyInstaller bincache was not moved out of place"
[[ ! -e "$HOME/.cache/ruff/0.9.1" ]] || fail "Ruff cache child was not moved out of place"
[[ -f "$HOME/.local/share/pyinstaller/config.json" ]] || fail "PyInstaller state file was removed"
trash_after=$(find "$HOME/.local/share/Trash/files" -mindepth 1 -maxdepth 1 | wc -l)
[[ $((trash_after - trash_before)) -eq 2 ]] || fail "expected two Trash entries (Trash went $trash_before -> $trash_after)"
grep -Fq 'CLEAN applied' "$HOME/.local/state/mole-linux/operations.log" || fail "oplog has no applied CLEAN row"
printf 'clean-dev-python-apply: ok\n'
