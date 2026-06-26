# Repository Guidelines

## Project Structure & Module Organization

XiaoLin is a Rust workspace with a Tauri desktop app. Core crates live in `crates/`, including `xiaolin-core`, `xiaolin-agent`, `xiaolin-gateway`, `xiaolin-session`, `xiaolin-memory`, and tool crates such as `xiaolin-tools-fs`. The desktop frontend is in `crates/xiaolin-app/src`, with Tauri backend code in `crates/xiaolin-app/src-tauri`. Channel integrations are under `extensions/`. Repository-level tests live in `tests/`, benchmark scenarios in `benchmarks/`, specs in `openspec/`, and docs in `docs/`.

## Build, Test, and Development Commands

- `cargo check --workspace`: type-check all Rust crates.
- `cargo test --workspace`: run Rust unit and integration tests.
- `cargo clippy --workspace --all-targets`: run Rust lints using workspace lint policy.
- `cargo fmt --all`: format Rust code.
- `cd crates/xiaolin-app && pnpm install`: install frontend dependencies.
- `cd crates/xiaolin-app && pnpm tauri dev`: run the recommended desktop development app.
- `cd crates/xiaolin-app && pnpm build`: type-check and build the frontend.
- `cd crates/xiaolin-app && pnpm test`: run Vitest tests.
- `cd crates/xiaolin-app && pnpm test:e2e`: run Playwright regression tests.

## Coding Style & Naming Conventions

Rust uses edition 2021 and workspace lints from `Cargo.toml`; `unsafe_code` is denied and Clippy `all` plus `pedantic` are warnings with selected allowances. Prefer `snake_case` modules/functions, `PascalCase` types, and crate-local modules that match existing boundaries. Frontend code is TypeScript/React; use existing component, store, and hook patterns in `crates/xiaolin-app/src`. Keep formatting tool-driven: `cargo fmt --all` for Rust and `pnpm build` for frontend checks.

## Testing Guidelines

Place Rust integration tests in each crate’s `tests/` directory and keep unit tests near the code they exercise. Use targeted commands, for example `cargo test -p xiaolin-agent` or `cargo test -p xiaolin-core skill`. Frontend tests use Vitest; browser regressions and visual tests use Playwright. Add or update tests when changing shared runtime behavior, tool execution, session storage, security policy, or visible UI flows.

## Commit & Pull Request Guidelines

Recent history follows Conventional Commits such as `fix(agent): ...` and `feat(browser): ...`. Use a scoped prefix (`feat`, `fix`, `docs`, `test`, `refactor`) and keep the subject imperative. Pull requests should describe the change, list verification commands, link issues or OpenSpec changes, and include screenshots for UI updates. Call out config, security, migration, or compatibility impacts.

## Security & Configuration Tips

Do not commit API keys or local secrets. User configuration belongs under `~/.xiaolin/config`, not in tracked files. Review `config/exec-policy.toml`, `deny.toml`, and sandbox-related crates before changing command execution, network access, or permission behavior.
