# AGENTS.md

## What this is

Rust + TypeScript NAPI module (napi-rs v2) that exposes Bun's `onBeforeParse` hook as a JS-friendly `jsBridge()` API. Lets users write bundler transforms in plain TS without per-project Rust code.

## Architecture (two files matter)

- `src/lib.rs` — Rust NAPI module. Exports `createBridge`, `releaseBridge`, and the `extern "C"` hook `bun_js_bridge_dispatch`. Blocks a Bun worker thread via `mpsc::sync_channel(0)` while waiting for the JS callback result.
- `js/index.ts` — TypeScript wrapper. `jsBridge(fn)` wraps a user's `TransformFn(source, path)` into a `NativePluginDescriptor`. Handles the CalleeHandled TSFN null-error-arg convention.

The `.node` binary is loaded via a try/catch cascade: `linux-x64-gnu` → `darwin-arm64` → `darwin-x64` → fallback to package require.

## Commands

```sh
bun run build          # napi build --platform --release (produces .node binary)
bun run build:debug    # napi build --platform (debug, no --release)
bun test               # runs __test__/basic.test.ts only
```

**Integration test** (manual, not in `bun test`): `bun run __test__/integration.ts` — writes to `/tmp/`, verifies the bridge fires in a real `Bun.build()` call.

## Prerequisites

- Bun >= 1.3.0
- Rust 1.70+ (for building the native module)
- `@napi-rs/cli` installed locally (already in devDependencies)

## Critical constraint: no event-loop-bound async in transforms

Transform functions **must not** await anything requiring the JS event loop to turn over (`fetch`, `Bun.file().text()`, `setTimeout`). The Rust side blocks a worker thread via a synchronous channel; if the JS callback needs the event loop, you get a deadlock.

**Safe:** CPU-only async — Babel, SWC, Oxc, `@code-inspector/core` (microtask-only resolution).

## Gotchas

- `releaseBridge()` is required after one-shot `Bun.build()` calls or the process never exits. Not needed with `Bun.serve()` (event loop stays alive).
- The TSFN uses `CalleeHandled` strategy — the native callback receives `(null, source, path)` per Node error-first convention. The JS wrapper strips the leading null so `TransformFn` sees `(source, path)`.
- `External::inner_from_raw(ptr)` must be used (not direct casting) — napi v2 External wraps data in a `TaggedObject`; direct casts segfault.
- `catch_unwind` around the `extern "C"` hook prevents Rust panics from crashing Bun — original source is returned unchanged on panic.
- The `.node` binary output name comes from `package.json` → `napi.binaryName` (currently `bun-js-beforeparse`). Platform suffix is appended by `napi build --platform`.

## Test structure

`__test__/basic.test.ts` — smoke tests only: module loads, `createBridge` returns a value, `jsBridge()` returns the correct descriptor shape. No actual `Bun.build()` invocation.

`__test__/integration.ts` — full end-to-end proof of concept. Writes a test `.tsx` file to `/tmp/bridge-test-src/`, builds with `onBeforeParse`, checks that the transform fires and the sentinel comment appears in output. Run manually.
