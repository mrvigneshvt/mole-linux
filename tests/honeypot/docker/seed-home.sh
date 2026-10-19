#!/bin/bash
set -euo pipefail
home=${1:?usage: seed-home.sh HOME}
manifest=/usr/local/share/mole-honeypot/manifest.tsv
repository=
linked_worktree=
tracked_files=()
while IFS=$'\t' read -r kind relative; do
    [[ -n "$kind" && "$kind" != \#* ]] || continue
    path="$home/$relative"
    case "$kind" in
        dir) mkdir -p "$path" ;;
        git-repo) repository="$relative" ;;
        git-track)
            tracked_files+=("$relative")
            mkdir -p "$(dirname "$path")"
            printf 'fn main() { println!("fixture project"); }\n' > "$path"
            ;;
        worktree) linked_worktree="$relative" ;;
        canary|target-file|authored-file|cargo-manifest)
            worktree_name=${linked_worktree##*/}
            if [[ "$kind" == canary && ( "$relative" == "$repository/.git/HEAD" || "$relative" == "$repository/.git/worktrees/$worktree_name/HEAD" ) ]]; then
                continue
            fi
            mkdir -p "$(dirname "$path")"
            case "$kind" in
                authored-file) printf '// authored local package, deliberately untracked\n' > "$path" ;;
                cargo-manifest) printf '[package]\nname = "honeypot-target"\nversion = "0.1.0"\nedition = "2024"\n' > "$path" ;;
                *) printf 'mole-linux fixture file: %s\n' "$relative" > "$path" ;;
            esac
            ;;
        sqlite-canary)
            mkdir -p "$(dirname "$path")"
            printf 'SQLite format 3\0mole-linux canary: %s\n' "$relative" > "$path"
            ;;
        *) printf 'unknown manifest kind: %s\n' "$kind" >&2; exit 1 ;;
    esac
done < "$manifest"
export HOME="$home" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
git -c init.defaultBranch=main init "$home/$repository"
git -C "$home/$repository" config user.name 'Honeypot Fixture'
git -C "$home/$repository" config user.email honeypot@example.invalid
for tracked_file in "${tracked_files[@]}"; do
    git -C "$home/$repository" add "${tracked_file#"$repository"/}"
done
git -C "$home/$repository" commit -m 'seed honeypot project'
git -C "$home/$repository" worktree add -b linked "$home/$linked_worktree"
checksum_file="$home/canary.sha256"
while IFS=$'\t' read -r kind relative; do
    [[ "$kind" == canary || "$kind" == sqlite-canary || "$kind" == git-track ]] || continue
    (cd "$home" && sha256sum "$relative") >> "$checksum_file"
done < "$manifest"
