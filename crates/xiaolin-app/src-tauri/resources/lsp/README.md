# Bundled LSP Binaries

Put platform-specific language server binaries in this folder before packaging.

Required file names:

- Linux: `rust-analyzer`
- macOS: `rust-analyzer`
- Windows: `rust-analyzer.exe`

Packaging behavior:

- These files are included into app bundle resources via `tauri.conf.json`.
- At runtime, XiaoLin tries bundled resources first.
- If not found, XiaoLin falls back to system `rust-analyzer` in PATH.

CI recommendation:

- Use `.github/scripts/fetch-rust-analyzer.sh <target> <out_dir>` in release pipeline.
- Place resulting binary into `crates/xiaolin-app/src-tauri/resources/lsp/`.
- Ensure executable bit is set on Unix targets.
