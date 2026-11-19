#!/usr/bin/env bash
# Runs the validation gates from AGENTS.md and prints GATES_OK when all pass.
#
#   tests/gates.sh            every gate, over the whole workspace
#   tests/gates.sh --changed  the gates the change since the parent commit
#                             needs, as AGENTS.md describes
#
# The tests run in the workspace's target directory, while lint, docs and
# the wasm check run beside them, each in a target directory of its own, so
# cargo's build lock does not make them wait for the tests. The consumer
# check builds in target/consumers already. Each log goes to
# target/gates/<step>.log, and a failing step prints its log.
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
logs="$target/gates"
mkdir -p "$logs"

wasm_crates=(smudge-core smudge-cpu smudge-platform smudge-text smudge-widgets smudge-markdown smudge-devtools smudge)

# The files the change touches: the working copy's change against its parent
# with jj, or the uncommitted and staged changes against HEAD with git.
changed_files() {
    if jj root >/dev/null 2>&1; then
        jj diff --name-only -r @
    else
        git diff --name-only HEAD
    fi
}

# Every workspace crate that depends on one of the crates named on standard
# input, directly or not, with those crates themselves.
with_dependents() {
    python3 -I -c '
import json, subprocess, sys
meta = json.loads(subprocess.run(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"],
    capture_output=True, text=True, check=True).stdout)
names = {package["name"] for package in meta["packages"]}
users = {name: set() for name in names}
for package in meta["packages"]:
    for dependency in package["dependencies"]:
        if dependency["name"] in names:
            users[dependency["name"]].add(package["name"])
todo = [line.strip() for line in sys.stdin if line.strip() in names]
seen = set(todo)
while todo:
    for user in users[todo.pop()]:
        if user not in seen:
            seen.add(user)
            todo.append(user)
print("\n".join(sorted(seen)))
'
}

full=true
tests=()
hot=true
consumers=true
wasm=true
frame=true
if [[ "${1:-}" == "--changed" ]]; then
    files="$(changed_files)"
    # A change outside the crates, docs and tests, such as the workspace
    # manifest, the toolchain or this script, needs every gate.
    if ! grep -qvE '^(crates/|docs/|tests/consumers/|README\.md|CHANGELOG\.md|AGENTS\.md)' <<<"$files"; then
        full=false
    fi
    if [[ -z "$files" ]]; then
        full=false
    fi
fi

if ! $full; then
    crates="$( (grep -oE '^crates/[^/]+' <<<"$files" | cut -d/ -f2; grep -q '^docs/' <<<"$files" && echo docs-check; grep -q '^README\.md' <<<"$files" && echo docs-check) | sort -u | with_dependents)"
    mapfile -t tests < <(grep -v '^$' <<<"$crates")
    # The hot reload suite builds fixture apps one at a time, so it runs when
    # the builder or the code a patch goes through changed.
    hot=false
    if grep -qE '^crates/cargo-smudge/|^crates/smudge-core/src/(hot|reactive/carry)|^crates/smudge-platform/src/hot|^crates/smudge/examples/hot' <<<"$files"; then
        hot=true
    fi
    consumers=false
    if grep -qE '(^|/)Cargo\.(toml|lock)$|^tests/consumers/|^crates/smudge-text/src/(fonts|system|lib)\.rs|^crates/smudge-text/assets/' <<<"$files"; then
        consumers=true
    fi
    wasm=false
    for crate in "${wasm_crates[@]}"; do
        if grep -qx "$crate" <<<"$crates"; then
            wasm=true
        fi
    done
    frame=false
    if grep -qx smudge-core <<<"$crates"; then
        frame=true
    fi
fi

failed=0
step() {
    local name="$1"
    shift
    if "$@" >"$logs/$name.log" 2>&1; then
        echo "ok    $name"
    else
        echo "FAIL  $name"
        tail -n 60 "$logs/$name.log"
        return 1
    fi
}

step fmt cargo fmt --all -- --check || failed=1

# The hot reload suite builds its fixture apps offline, from lockfiles of
# their own, so their dependencies are fetched first.
fetch_fixtures() {
    local fixture
    for fixture in crates/cargo-smudge/tests/fixtures/*/Cargo.toml; do
        cargo fetch --locked --manifest-path "$fixture" || return 1
    done
}

run_tests() {
    if $hot; then
        fetch_fixtures || return 1
    fi
    if $full; then
        cargo test --workspace --all-features || return 1
    else
        local args=() cli=false
        for crate in "${tests[@]}"; do
            if [[ $crate == cargo-smudge ]]; then
                cli=true
            else
                args+=(-p "$crate")
            fi
        done
        if ((${#args[@]})); then
            cargo test "${args[@]}" --all-features || return 1
        fi
        if $hot; then
            cargo test -p cargo-smudge --all-features || return 1
        elif $cli; then
            # The builder changed only where the hot reload suite does not
            # reach, so its unit tests and every other test file run.
            local files=(--bins)
            for file in crates/cargo-smudge/tests/*.rs; do
                file="$(basename "$file" .rs)"
                if [[ $file != dev ]]; then
                    files+=(--test "$file")
                fi
            done
            cargo test -p cargo-smudge --all-features "${files[@]}" || return 1
        fi
    fi
    if $frame; then
        cargo test --release -p smudge-core --test frame || return 1
    fi
}

lint() {
    CARGO_TARGET_DIR="$target/gate-lint" CARGO_BUILD_JOBS=4 \
        cargo clippy --workspace --all-targets --all-features -- -D warnings || return 1
    CARGO_TARGET_DIR="$target/gate-lint" CARGO_BUILD_JOBS=4 RUSTDOCFLAGS="-D warnings" \
        cargo doc --workspace --no-deps --all-features
}

check_wasm() {
    local args=()
    for crate in "${wasm_crates[@]}"; do
        args+=(-p "$crate")
    done
    CARGO_TARGET_DIR="$target/gate-wasm" CARGO_BUILD_JOBS=4 RUSTFLAGS="-D warnings" \
        cargo check --target wasm32-unknown-unknown "${args[@]}"
}

step tests run_tests &
pids=($!)
step lint lint &
pids+=($!)
if $wasm; then
    step wasm check_wasm &
    pids+=($!)
fi
if $consumers; then
    step consumers env CARGO_BUILD_JOBS=4 tests/consumers/check.sh &
    pids+=($!)
fi
for pid in "${pids[@]}"; do
    wait "$pid" || failed=1
done

if grep -q 'source = "git+' Cargo.lock; then
    echo "FAIL  lockfile: a git source"
    failed=1
fi

if $full; then
    echo "scope: every gate"
else
    echo "scope: tests of ${tests[*]:-nothing}; hot reload suite $hot; wasm $wasm; consumers $consumers"
fi
if ((failed)); then
    exit 1
fi
echo GATES_OK
