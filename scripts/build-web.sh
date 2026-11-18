#!/usr/bin/env bash
# Builds the WebAssembly bundle into dist/ (what GitHub Pages serves).
set -euo pipefail
cd "$(dirname "$0")/.."
export BUILD_COMMIT="${GITHUB_SHA:-$(git rev-parse HEAD 2>/dev/null || echo local)}"
cargo build --locked --release --target wasm32-unknown-unknown --bin space-game
WASM_BINDGEN_VERSION=$(python3 - <<'PY'
import tomllib
with open('Cargo.lock', 'rb') as lock:
    packages = tomllib.load(lock)['package']
print(next(p['version'] for p in packages if p['name'] == 'wasm-bindgen'))
PY
)
if ! command -v wasm-bindgen >/dev/null || [[ "$(wasm-bindgen --version)" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]]; then
  cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION" --locked
fi
rm -rf dist
package_dir="dist/pkg/${BUILD_COMMIT}"
mkdir -p "$package_dir"
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section --out-dir "$package_dir" --out-name space_game \
  target/wasm32-unknown-unknown/release/space-game.wasm
cp web/index.html dist/index.html
cp assets/fonts/OFL.txt dist/FONT_LICENSE.txt
python3 - <<'PY'
import os
from pathlib import Path
p = Path('dist/index.html')
build = os.environ['BUILD_COMMIT']
p.write_text(p.read_text().replace('__BUILD_SHA__', build[:7]).replace('__BUILD_ID__', build))
PY
touch dist/.nojekyll
du -sh "$package_dir"/*.wasm
