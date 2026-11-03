# Development

## Layout

| Path | Purpose |
|---|---|
| `src/` | React frontend, bundled by esbuild (`scripts/build-frontend.mjs`) into `dist/index.js`. React, react-dom and `@decky/ui` come from Steam's own copies (`window.SP_REACT`, `window.SP_REACTDOM`, `window.DFL`), and `@decky/manifest` is `plugin.json`. |
| `backend-rust/` | Native backend (`bin/backend` in the package), built on the `decky-plugin` crate from [beallio/decky-loader](https://github.com/beallio/decky-loader) |
| `main.py` | Byte-identical copy of `decky-plugin/shim/main.py`. The upstream Python loader imports it, and it starts `bin/backend`. The Rust loader ignores it and starts `bin/backend` itself. |
| `scripts/package.sh` | Makes `out/decky-storage-cleaner-rust.zip` |
| `scripts/quality-gates` | Every check that CI runs |

## Plugin identity

Decky identifies an installed plugin, and matches store updates to it, by `plugin.json` `name`. This port is named `Storage Cleaner (Rust)` so that it is a separate plugin from the official `Storage Cleaner` (mcarlucci): store catalogs that merge entries by name, such as decky-plugins-extended, list it as its own card and never offer it as an update to the original. Do not rename it again: a new name is a new plugin to Decky, and existing installs would not update. The store card author comes from `package.json` `author`.

## Tools

Node 24, pnpm 10.33 (`packageManager` in `package.json`), and rustup. `rust-toolchain.toml` selects Rust 1.98.1 with the `x86_64-unknown-linux-musl` target, so the backend is a static executable that SteamOS updates cannot break. No C toolchain is necessary.

```sh
pnpm install
scripts/quality-gates   # fmt, clippy, tests, version sync, shim parity, typecheck, pnpm audit --prod, package
pnpm package            # only build out/decky-storage-cleaner-rust.zip
pnpm watch              # rebuild dist/index.js, with an inline source map, on every change
```

## Dependency security

- `.github/workflows/audit.yml` runs on every push and pull request, every day at 10:00 UTC, and before each release. It runs `cargo deny check advisories bans sources` (`backend-rust/deny.toml`: RustSec advisories, unmaintained and yanked crates, wildcard versions, and only crates.io plus the pinned `beallio/decky-loader` git dependency) and `pnpm audit --prod`, which both fail the run. A full `pnpm audit` then reports advisories in build tools (esbuild, TypeScript) without failing, because those never ship in the plugin. Licenses are not checked.
- `scripts/quality-gates` also runs `pnpm audit --prod`, so a known vulnerability in JavaScript that ships in the plugin fails a local build.
- `.github/dependabot.yml` opens weekly grouped update pull requests for Cargo, npm and GitHub Actions. It skips `decky-plugin`: update it by hand (below), because `main.py` must change with it.

## Backend API

| Method | Arguments | Result |
|---|---|---|
| `scan` | `kind`: `"shadercache"` or `"compatdata"` | `{ entries: [{ appid, bytes }] }`, largest first |
| `delete` | `kind`, `appids: string[]` | `{ deleted: string[], failed: [{ appid, error }] }` |

The frontend never sends a path. The backend accepts only the two kinds and all-digit app IDs, lists only real folders, and refuses a symbolic link or a file, so a delete cannot leave `~/.steam/steam/steamapps/<kind>`. If a delete fails with "permission denied" (some prefixes contain read-only folders), the backend gives the owner full access to every folder inside the app folder, without following symbolic links, and tries once more. Sizes are allocated disk space (like `du`): symbolic links are not followed and a hard-linked file counts once.

`scan` measures the app folders in parallel, one worker per core. It reads each folder through its own descriptor with `rustix` (`openat`, `statat`, and `RawDir`, which reads names into one buffer per folder) instead of `std::fs::DirEntry::metadata`: on musl, std looks up the full path again for every file, which made warm scans about 6 times slower on the VM. The backend has no `unsafe` code (`#![forbid(unsafe_code)]`). After a delete with no failures, the frontend removes the deleted entries without a new scan; after any failure it scans again, because a failed delete can leave part of a folder.

The frontend shows only apps that Steam can name. It asks for app details only when `appStore.GetAppOverviewByAppID(appid)` returns an overview: Steam never answers a details request for some IDs, such as `0`, so each of those would hold the list for the full 1 s timeout. Every game, tool and non-Steam shortcut in the library has an overview. The frontend then groups entries: `BIsShortcut()` on the overview marks a non-Steam shortcut, and otherwise `iInstallFolder === -1` marks a Steam game that is not installed. `iInstallFolder` alone cannot separate the two, because Steam returns `-1` for both.

## Updating the SDK

1. Change `rev` for `decky-plugin` in `backend-rust/Cargo.toml` and run `cargo update -p decky-plugin --manifest-path backend-rust/Cargo.toml`.
2. Copy the new shim: `scripts/quality-gates` fails until `main.py` equals `shim/main.py` at the pinned rev.

To build against a local `decky-loader` checkout, uncomment the `[patch]` section in `backend-rust/Cargo.toml`.

## Release

Set `version` in `package.json` and `backend-rust/Cargo.toml` (`scripts/quality-gates` fails if they differ), then push a `vX.Y.Z` tag. The Release workflow runs the quality gates and attaches the ZIP and its SHA-256 to a GitHub Release.
