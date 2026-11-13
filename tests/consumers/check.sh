#!/bin/sh
# The consumers in this directory stand outside the workspace and pin down
# the cargo feature unification the font and accessibility features rely on:
# a direct smudge-widgets consumer finds a font with its defaults, a font
# enabled by another dependency reaches the facade, a library that forwards
# the font choice builds on native and wasm, the machine's fonts
# alone draw text, a headless build needs no window, every configuration the
# small-binary guide documents builds on native, the bundled-font ones build on
# wasm, wasm with system fonts alone fails, and a build with no font source
# fails instead of drawing no text. Run it from anywhere.
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

for dir in */; do
    if grep -q 'source = "git+' "$dir/Cargo.lock"; then
        echo "$dir resolved a git dependency" >&2
        exit 1
    fi
done
echo CONSUMERS_OK
