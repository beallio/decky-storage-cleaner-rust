#!/usr/bin/env bash
# Packages the built frontend and musl backend into out/decky-storage-cleaner-rust.zip.
# Run `pnpm build` and `pnpm build:backend` first (`pnpm package` does both).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

name=decky-storage-cleaner-rust
target_dir="$(cargo metadata --manifest-path backend-rust/Cargo.toml --format-version 1 --no-deps |
  node -p 'JSON.parse(require("fs").readFileSync(0)).target_directory')"
backend="$target_dir/x86_64-unknown-linux-musl/release/backend"

# SteamOS updates can replace system libraries, so the backend must not load any.
if readelf -l "$backend" | grep -q INTERP; then
  echo "package: $backend is dynamically linked" >&2
  exit 1
fi

stage="$(mktemp -d "${TMPDIR:-/var/tmp}/$name.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name/bin" "$stage/$name/dist" out
cp plugin.json package.json main.py LICENSE README.md "$stage/$name/"
cp dist/index.js "$stage/$name/dist/"
install -m 0755 "$backend" "$stage/$name/bin/backend"

rm -f "out/$name.zip"
(cd "$stage" && zip -qrX "$OLDPWD/out/$name.zip" "$name")
echo "package: out/$name.zip"
