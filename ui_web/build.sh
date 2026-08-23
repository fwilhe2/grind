#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build the browser shell into ui_web/dist.
#
# Two steps, because the Rust compiler only produces half of what a page needs: a
# `.wasm` module, and then the `wasm-bindgen` CLI to write the JavaScript glue that
# loads it and marshals values across. The CLI's version must match the
# `wasm-bindgen` crate the module was compiled against — a mismatch fails with a
# schema error, which is why this script checks first.
#
#   ui_web/build.sh [debug|release]
#
# Prerequisites (see .github/workflows/web.yml for the CI equivalent):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the version printed below>

set -euo pipefail

profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dist="$root/ui_web/dist"

case "$profile" in
    debug)   cargo_flags=() ;;
    release) cargo_flags=(--release) ;;
    *) echo "usage: $(basename "$0") [debug|release]" >&2; exit 2 ;;
esac

# The version cargo actually resolved, so the check below is never out of date.
# The exact-match pattern is what keeps wasm-bindgen-futures from answering.
wanted="$(grep -A1 '^name = "wasm-bindgen"$' "$root/Cargo.lock" \
    | sed -n 's/^version = "\(.*\)"/\1/p' | head -n1)"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "wasm-bindgen CLI not found — run: cargo install wasm-bindgen-cli --version $wanted" >&2
    exit 1
fi

have="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$have" != "$wanted" ]; then
    echo "wasm-bindgen CLI is $have but the crate is $wanted; the glue would not load." >&2
    echo "run: cargo install wasm-bindgen-cli --version $wanted" >&2
    exit 1
fi

cargo build -p grind-web --target wasm32-unknown-unknown "${cargo_flags[@]}"

rm -rf "$dist"
mkdir -p "$dist"

wasm-bindgen \
    --target web \
    --no-typescript \
    --out-dir "$dist" \
    --out-name grind_web \
    "$root/target/wasm32-unknown-unknown/$profile/grind_web.wasm"

# The page is static; it only ever needed the module next to it.
cp "$root/ui_web/index.html" "$root/ui_web/style.css" "$dist/"

echo "ui_web/dist is ready — serve it, do not open index.html from disk:"
echo "  python3 -m http.server --directory ui_web/dist 8000"
