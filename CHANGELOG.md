# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-09

### Added

- Initial release of `bun-js-beforeparse`
- `jsBridge(fn)` — wraps a TypeScript transform function for use as a Bun `onBeforeParse` plugin
- `releaseBridge(descriptor)` — releases the TSFN reference so the process can exit after `Bun.build()`
- Pre-built `.node` binaries for 8 platforms:
  - Linux x64 (glibc + musl)
  - Linux arm64 (glibc + musl)
  - macOS x64 (Intel)
  - macOS arm64 (Apple Silicon)
  - Windows x64 (MSVC)
  - Windows arm64 (MSVC)
- CI workflow: lint + test matrix (Ubuntu, macOS, Windows)
- Release workflow: automated cross-platform builds + npm publish on semver tag push
- MIT license

### Fixed

- Dynamic platform-aware binary loading (replaced hardcoded try/catch cascade)
- Windows binary resolution
- `prepublishOnly` hook conflict with CI builds
- Repository URL placeholder in `package.json`

### Changed

- Replaced `napi-rs/action` with direct `napi build` commands in release workflow
- Removed `macos-13` (Intel) runner from CI (unavailable)
- Added `mwilli00/setup-zig@v1` for musl cross-compilation
- Added `x86_64-apple-darwin` cross-compilation from arm64 runner
