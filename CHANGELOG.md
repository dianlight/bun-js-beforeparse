# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- End-to-end `Bun.build()` tests for both sync and async transforms in `__test__/basic.test.ts` (7 new tests: 4 sync, 3 async). The test suite now invokes the full `onBeforeParse` round-trip and asserts that the transformed source appears in the bundle.
- `beforeAll` warmup build in the test suite — the first `Bun.build()` with a native `onBeforeParse` plugin needs native-plugin initialization; without a warmup, the first real test silently receives the original source.
- Cleanup test that removes the `/tmp/bun-js-beforeparse-test` scratch directory after the suite runs.

### Fixed

- **Async transforms silently dropped their result.** The TSFN callback was typed `Function<..., String>`, so when an `async` transform returned a `Promise`, napi-rs failed with `StringExpected, Failed to convert JavaScript value Object {} into rust type String` and the worker thread fell back to the original source. The callback return type is now `Unknown<'static>`; at runtime the dispatch hook inspects the value type via `value.get_type()`:
  - `ValueType::String` → sync path; extract the string and send it through the rendezvous channel directly.
  - Otherwise (Promise) → async path; cast to `PromiseRaw<String>` and wire `.then()` / `.catch()` so the resolved value reaches the blocked worker thread after microtask resolution. CPU-only async (Babel, SWC, Oxc, `await Promise.resolve()`) is safe; event-loop-bound async still deadlocks.
- Four napi-rs v3 compilation errors in `src/lib.rs`:
  - `JsValueType` does not exist in v3 → `napi::ValueType`.
  - `Unknown::cast()` is now `unsafe` in v3 → wrapped in `unsafe { ... }`.
  - `env.create_function()` expects `extern "C"` fn, not closures → replaced with `PromiseRaw::then()` / `.catch()`.
  - `Unknown` has no `.call()` method in v3 → replaced with the `PromiseRaw` API.
- Stale doc comment on `createBridge` in `index.d.ts`: it claimed "Return type is String" but the regenerated binding now returns `unknown` (String | Promise<String>). The doc comment now describes both paths.

### Changed

- TSFN `Return` type parameter changed from `String` to `Unknown<'static>` in `BridgeFn` so the runtime can accept both sync (`String`) and async (`Promise<String>`) transform results.
- Doc comment on `create_bridge` in `src/lib.rs` now reads: "Return type is String (sync) or Promise<String> (async) — both are accepted; the dispatch hook inspects the runtime type and wires Promise results back via .then()/.catch()." Regenerated `index.d.ts` reflects this.

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
