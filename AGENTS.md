# AGENTS.md — anyNFS.xyz-desktop

## What this is

Rust + Tauri v2 desktop app, scaffolded with `cargo tauri init` (tauri-cli `2.12.4`).
Single Rust crate lives in `src-tauri/` (`src-tauri/Cargo.toml`, package `app`).
Frontend lives at repo root (Vite), output `dist/`. `tauri.conf.json` points
`frontendDist` at `../dist` and runs `npm run dev` / `npm run build`.

## Setup (security constraints — do not skip)

- Node.js + npm are **not** installed system-wide. They are vendored in this repo:
  `vendor/node-v26.13.0-linux-x64/`.
- Before any node/cargo command, source the in-repo PATH shim:

```
source env.sh                       # adds vendor/.../bin to PATH
export PATH="$HOME/.cargo/bin:$PATH"  # for `cargo tauri` (tauri-cli binary)
```

- `env.sh` only touches this project root. No `pacman`, no `sudo`, nothing outside the repo.

## Verified commands

Run from repo root (after `source env.sh` + `$HOME/.cargo/bin` on PATH):

| Command | What it does | Notes |
|---|---|---|
| `npm run dev` | Vite dev server (`:1420`) | used by `cargo tauri dev` via `beforeDevCommand` |
| `npm run build` | Vite build → `dist/` | used by `cargo tauri build` via `beforeBuildCommand` |
| `cargo tauri dev` | dev server + Rust app | **first run compiles ~433 crates — slow, one-time** |
| `cargo tauri build` | bundle production app | runs `npm run build` first |
| `cargo build --manifest-path src-tauri/Cargo.toml` | compile Rust only | no root `Cargo.toml`; crate is in `src-tauri/` |

## Entrypoints & wiring

- `src-tauri/src/main.rs` — Rust app entry
- `src-tauri/src/lib.rs` — `app_lib` (staticlib/cdylib/rlib)
- `src-tauri/tauri.conf.json` — Tauri config (schema `/config/2`)
- `src-tauri/build.rs` — `tauri_build::build()`
- `index.html` + `vite.config.js` + `src/main.js` — frontend (root)
- `package.json` scripts: `dev`, `build`, `test` (no real test command yet)

## Conventions / gotchas

- No root `Cargo.toml`: always use `--manifest-path src-tauri/Cargo.toml` or `cd src-tauri`.
- `cargo tauri dev`/`build` run npm scripts from repo root; `dist/` must exist for production builds (`npm run build` first).
- First Tauri command compiles the toolchain (~1–2 min). Subsequent runs are fast.
- Tests: not configured (`npm test` and `cargo test` are placeholders / not set up).
- `.gitignore` excludes vendored node, `target/`, `node_modules/`, `dist/`, `src-tauri/target/`, `src-tauri/gen/`.

## Sources of truth (verified)

- `vendor/node-v26.13.0-linux-x64/bin/node --version` → `v26.13.0`
- `vendor/node-v26.13.0-linux-x64/bin/npm --version` → `11.20.0`
- `cargo-tauri --version` → `tauri-cli 2.12.4`
- `npm run build` → `dist/` (verified)
- `cargo tauri dev` → starts dev + Rust compile (verified)
