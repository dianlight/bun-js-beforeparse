# AGENTS.md

## What this is

Rust + TypeScript NAPI module (napi-rs v3) that exposes Bun's `onBeforeParse` hook as a JS-friendly `jsBridge()` API. Lets users write bundler transforms in plain TS without per-project Rust code.

## Architecture (two files matter)

- `src/lib.rs` — Rust NAPI module. Exports `createBridge`, `releaseBridge`, and the `extern "C"` hook `bun_js_bridge_dispatch`. Blocks a Bun worker thread via `mpsc::sync_channel(0)` while waiting for the JS callback result. The TSFN return type is `Unknown<'static>` so both sync (`String`) and async (`Promise<String>`) transforms work — `dispatch_inner` inspects the runtime type and either extracts the string directly or wires `PromiseRaw::then()` / `.catch()` to send the resolved value through the channel.
- `js/index.ts` — TypeScript wrapper. `jsBridge(fn)` wraps a user's `TransformFn(source, path)` into a `NativePluginDescriptor`. With `callee_handled = false` there is no null error-first arg, so the wrapper passes `fn` through unchanged.

The `.node` binary is loaded via a try/catch cascade: `linux-x64-gnu` → `darwin-arm64` → `darwin-x64` → fallback to package require.

## Release workflow gotcha

`napi pre-publish` only publishes the 8 platform stub packages (e.g. `bun-js-beforeparse-linux-x64-gnu`). It does **not** publish the root `bun-js-beforeparse` package. The release workflow must include a separate `npm publish --provenance --access public` step for the root package, placed after `napi pre-publish` and before `action-gh-release`.

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

- `releaseBridge()` is a no-op in napi v3 (`Weak=true` TSFN does not hold the event loop open), but it is preserved for API compatibility. Calling it is safe.
- The TSFN uses `callee_handled = false` — the JS callback is invoked as `fn(source, path)` directly with **no null error-first arg**. The `jsBridge()` wrapper does not strip a leading null; it passes `fn` through unchanged. (This diverges from the old napi-rs default and from the v0.1.0 behavior.)
- Async transforms return a `Promise<String>` — the dispatch hook must branch on `value.get_type()` (sync `String` → extract; otherwise cast to `PromiseRaw<String>` and wire `.then()` / `.catch()`). If you change the TSFN `Return` type back to `String`, async transforms silently fail with `StringExpected, Failed to convert JavaScript value Object {} into rust type String`.
- `External::inner_from_raw(ptr)` must be used (not direct casting) — napi v3 External wraps data in a `TaggedObject`; direct casts segfault.
- `Unknown::cast()` is `unsafe` in napi-rs v3 — wrap in `unsafe { ... }`.
- `catch_unwind` around the `extern "C" hook` prevents Rust panics from crashing Bun — original source is returned unchanged on panic.
- The `.node` binary output name comes from `package.json` → `napi.binaryName` (currently `bun-js-beforeparse`). Platform suffix is appended by `napi build --platform`.
- Tests must modify an **exported** value in the transform (in-place string replacement works) rather than prepending `var X = true;` or `// comments`. Bun's bundler tree-shakes unused declarations and strips non-source-map comments from the bundle, so prepended sentinels disappear from the output.
- The first `Bun.build()` with `onBeforeParse` in a process needs native-plugin initialization, so the warmup in `beforeAll` is load-bearing — without it, the first real test silently receives the original untransformed source back.

## Test structure

`__test__/basic.test.ts` — full test suite (13 tests). Three groups:
1. **Smoke tests** — module loads, `createBridge` returns a value, `jsBridge()` returns the correct descriptor shape.
2. **Sync transform tests** (`jsBridge sync transform` describe block) — real `Bun.build()` round-trips: sentinel replacement appears in the bundle, correct `(source, path)` args received, in-place edit works, empty-string return falls back to original source.
3. **Async transform tests** (`jsBridge async transform` describe block) — `async` transforms with `await Promise.resolve()` and chained microtasks resolve and reach the bundled output. Confirms the `PromiseRaw` async dispatch path.

A `beforeAll` warmup build is required before the first transform test — the first `Bun.build()` with a native `onBeforeParse` plugin needs native-plugin initialization. Tests write scratch files under `/tmp/bun-js-beforeparse-test/`, which a final cleanup test removes.

The suite is included in `bun test`.

`__test__/integration.ts` — full end-to-end proof of concept. Writes a test `.tsx` file to `/tmp/bridge-test-src/`, builds with `onBeforeParse`, checks that the transform fires and the sentinel comment appears in output. Run manually (`bun run __test__/integration.ts`). Not part of `bun test` due to its `process.exit()` usage.
