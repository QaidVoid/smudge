#!/bin/sh
# The consumers in this directory stand outside the workspace and pin down
# the cargo feature unification the font and accessibility features rely on:
# a direct smudge-widgets consumer finds a font with its defaults, a font
# enabled by another dependency reaches the facade, a library that forwards
# the font choice builds on native and wasm, the machine's fonts
# alone draw text, a headless build needs no window, every configuration the
# small-binary guide documents builds on native, the bundled-font ones build on
# wasm, wasm with system fonts alone fails, and a build with no font source
# fails instead of drawing no text. It also reads the IR of generic reactive
# and text code compiled in an application crate, which must reach the
# runtime and the text system through calls into the core and the text crate
# and never address their thread-locals, because a hot patch built from that
# crate would otherwise carry empty copies of its own. Run it from anywhere.
set -e
# A relative target directory would resolve inside each consumer, so make
# it absolute against the caller's directory first.
case "${CARGO_TARGET_DIR:-}" in
    "" | /*) ;;
    *) CARGO_TARGET_DIR="$(pwd)/$CARGO_TARGET_DIR" ;;
esac
cd "$(dirname "$0")"
root="$(cd ../.. && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}/consumers"
mkdir -p "$CARGO_TARGET_DIR"
log="$CARGO_TARGET_DIR/consumer.log"

# Start every consumer from the workspace's resolved versions, so a new
# upstream release cannot change what the gate builds.
for dir in */; do
    cp "$root/Cargo.lock" "$dir/Cargo.lock"
done

expect_fail() {
    name="$1"
    shift
    if "$@" 2>"$log"; then
        echo "$name built, but it must fail" >&2
        exit 1
    fi
    grep -q 'smudge-text has no font source' "$log" || {
        echo "$name failed for another reason:" >&2
        cat "$log" >&2
        exit 1
    }
}

echo 'consumer: smudge-widgets defaults provide fonts'
( cd widgets-fonts && cargo test -q )

echo 'consumer: a font from another dependency reaches the facade'
( cd fonts-from-elsewhere && cargo test -q )

echo 'consumer: the machine'"'"'s fonts alone draw text'
( cd system-fonts-only && cargo test -q )

echo 'consumer: a headless build needs no window'
( cd headless && cargo check -q )

echo 'consumer: a library that forwards the font choice builds on its own'
( cd library && cargo check -q && cargo check -q --target wasm32-unknown-unknown )

echo 'consumer: the facade with no font feature fails to compile'
expect_fail no-font-fails sh -c 'cd no-font-fails && cargo build -q'

echo 'consumer: the guide configurations build'
( cd guide-app && cargo check -q )
( cd guide-app && cargo check -q --no-default-features --features bundled-fonts )
( cd guide-app && cargo check -q --no-default-features --features a11y,system-fonts )
( cd guide-app && cargo check -q --no-default-features --features system-fonts )
( cd guide-app && cargo check -q --features system-fonts )
expect_fail 'guide-app without a font' sh -c 'cd guide-app && cargo check -q --no-default-features'

echo 'consumer: the wasm font rule'
( cd guide-app && cargo check -q --target wasm32-unknown-unknown )
( cd guide-app && cargo check -q --target wasm32-unknown-unknown --no-default-features --features bundled-fonts )
( cd headless && cargo check -q --target wasm32-unknown-unknown )
expect_fail 'guide-app on wasm with system fonts only' sh -c 'cd guide-app && cargo check -q --target wasm32-unknown-unknown --no-default-features --features system-fonts'

echo 'consumer: application code reaches the runtime and the text system through calls'
for profile in dev release; do
    # The IR goes to a path of its own, removed first, so the check reads what
    # this build emitted whatever the target, never a file left by an earlier
    # one. One codegen unit makes rustc write one file there. The pass with
    # the hot-reload feature covers a wrapped closure: its shim must be
    # compiled here, and its lookup must go through the non-generic,
    # non-inlined getter of the core.
    for set in "" "-hot"; do
        if [ "$set" = "-hot" ]; then
            features="--features hot-reload"
        else
            features=""
        fi
        ir="$CARGO_TARGET_DIR/runtime-entry-$profile$set.ll"
        rm -f "$ir"
        touch runtime-entry/src/lib.rs
        # shellcheck disable=SC2086
        ( cd runtime-entry && cargo rustc -q --profile "$profile" $features --lib -- -C codegen-units=1 --emit=llvm-ir="$ir" )
        [ -f "$ir" ] || {
            echo "the $profile$set build of runtime-entry emitted no IR at $ir" >&2
            exit 1
        }
        # LLVM may put attributes and a model between `thread_local` and
        # `global`, so any global declaration of a thread-local whose symbol
        # names either crate counts.
        tls='^@[^ ]*smudge_(core|text)[^ ]* = .*\bthread_local\b'
        if grep -Eq "$tls" "$ir"; then
            echo "the $profile$set IR of runtime-entry addresses a thread-local of the core or the text crate:" >&2
            grep -E "$tls" "$ir" >&2
            exit 1
        fi
        grep -q 'call.*smudge_core8reactive7runtime6notify' "$ir" || {
            echo "the $profile$set IR of runtime-entry never calls into the runtime, so it checks nothing" >&2
            exit 1
        }
        grep -q 'call.*smudge_text6system12with_current' "$ir" || {
            echo "the $profile$set IR of runtime-entry never calls into the text system, so it checks nothing" >&2
            exit 1
        }
        if [ "$set" = "-hot" ] && [ "$(uname -s)" = "Linux" ]; then
            grep -q 'call.*smudge_core3hot3imp9published' "$ir" || {
                echo "the $profile IR of runtime-entry reaches the jump table without the non-inlined published getter of the core, so a patch would read a stale copy of the table" >&2
                exit 1
            }
            grep -q 'define .*smudge_core3hot3imp11shim_shared' "$ir" || {
                echo "the $profile IR of runtime-entry holds no shim of the wrapped closure, so a patch of this crate would never map it" >&2
                exit 1
            }
        fi
    done
done

for dir in */; do
    if grep -q 'source = "git+' "$dir/Cargo.lock"; then
        echo "$dir resolved a git dependency" >&2
        exit 1
    fi
done
echo CONSUMERS_OK
