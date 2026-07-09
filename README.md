# bun-js-beforeparse

Use any JavaScript/TypeScript function as a [Bun](https://bun.sh) bundler
[`onBeforeParse`](https://bun.sh/docs/bundler/plugins#onbeforeparse) plugin — no
per-project Rust code required.

## The problem this solves

Bun 1.3.x JS bundler plugins **cannot intercept native file types** (`.tsx`, `.jsx`,
`.ts`, `.js`) via `onLoad`/`onResolve`. Only compiled native NAPI modules can intercept
those files using the `onBeforeParse` hook. This package wraps that native hook once, so
you can write your transforms in plain TypeScript.

## Installation

```sh
bun add bun-js-beforeparse
```

Pre-built `.node` binaries are included for:

| Platform | Architecture |
|---|---|
| Linux | x64 (glibc) |
| macOS | x64 (Intel) |
| macOS | arm64 (Apple Silicon) |
| Windows | x64 (MSVC) |

## Quick start

```ts
import { jsBridge } from "bun-js-beforeparse";

const server = Bun.serve({
  routes: { "/": homepage },
  plugins: [
    {
      name: "my-transform",
      setup(build) {
        build.onBeforeParse(
          { filter: /\.[jt]sx$/, namespace: "file" },
          jsBridge(async (source, path) => {
            // Anything here — plain TypeScript, no Rust
            return source.replace(/foo/g, "bar");
          }),
        );
      },
    },
  ],
  development: { hmr: true },
  port: 3000,
});
```

For one-shot `Bun.build()` calls, release the bridge when done so the process can exit:

```ts
import { jsBridge, releaseBridge } from "bun-js-beforeparse";

const bridge = jsBridge(myTransform);

await Bun.build({
  entrypoints: ["./src/index.tsx"],
  plugins: [{
    name: "transform",
    setup(build) {
      build.onBeforeParse({ filter: /\.[jt]sx$/ }, bridge);
    },
  }],
});

releaseBridge(bridge); // allows the process to exit
```

## API

### `jsBridge(fn)`

Wraps a TypeScript transform function for use as a Bun `onBeforeParse` plugin.

```ts
function jsBridge(fn: TransformFn): NativePluginDescriptor
```

- **`fn`** — Your transform. Receives `(source: string, path: string)` and must return
  the (possibly modified) source as a `string` or `Promise<string>`.
- **Returns** the descriptor object `{ napiModule, symbol, external }` expected by
  `build.onBeforeParse(matcher, HERE)`.

### `releaseBridge(descriptor)`

Releases the TSFN reference so the event loop can exit after a `Bun.build()` call.
Not needed when using `Bun.serve()` (the server keeps the event loop alive anyway).

```ts
function releaseBridge(descriptor: NativePluginDescriptor): void
```

### `TransformFn`

```ts
type TransformFn = (source: string, path: string) => string | Promise<string>
```

## Constraint: no event-loop-bound async

Your transform **must not** `await` anything that requires the JS event loop to yield
(e.g. `await fetch(...)`, `await Bun.file(...).text()`).

**Safe:** CPU-only async work — Babel transforms, SWC, Oxc, `@code-inspector/core`.
These resolve through microtasks without yielding, so the blocked worker thread unblocks
as soon as the microtask queue drains.

**Unsafe:** Anything that needs a new I/O event — `fetch`, `Bun.file().text()`,
`setTimeout`-based delays, anything backed by libuv/tokio callbacks.

**Why:** The bridge blocks a Bun bundler worker thread via a synchronous Rust channel
(`mpsc::sync_channel(0)`) while it waits for the JS callback to send back the result.
If the callback needs the event loop to turn over (e.g. awaiting a fetch response), and
the event loop is blocked handling the TSFN callback, you get a deadlock.

## How it works

```
Bun worker thread (native)           JS main thread
──────────────────────────           ──────────────
bun_js_bridge_dispatch()             TSFN callback fires
  OnBeforeParse::from_raw()            call_with_return_value cb
  read source bytes (zero-copy)        calls user's JS fn(source, path)
  create SyncChannel(0)                user fn returns/resolves string
  tsfn.call_with_return_value(         coerce_to_string → send through channel
    payload, Blocking, cb)           ─────────────────────────────────────────
  ←─── blocks on rx.recv() ──────────── tx.send(transformed_source)
  handle.set_output_source_code()
```

Key design decisions:

- **`mpsc::sync_channel(0)`** — a rendezvous channel. `send()` blocks until `recv()`
  picks up, so the worker thread blocks exactly until the JS result is ready.
- **`Mutex<ThreadsafeFunction>`** — the TSFN itself needs `&mut self` for `unref()`, so
  it is wrapped in a `Mutex`. Concurrent worker threads share it via `Arc`.
- **`CalleeHandled` TSFN strategy** — napi-rs's default. It prepends a null "error" arg
  following Node.js error-first callback convention: `callback(null, source, path)`. The
  `jsBridge()` wrapper automatically skips that first null, so `TransformFn` cleanly
  receives `(source, path)`.
- **`External::<Arc<BridgeFn>>::inner_from_raw(ptr)`** — napi v2 `External<T>` wraps
  data in a `TaggedObject<T>` struct, not a bare `*mut T`. Direct casting would segfault;
  `inner_from_raw` navigates the wrapper correctly.
- **`catch_unwind` in the `extern "C" hook`** — prevents a Rust panic from crashing the
  Bun runtime. On panic the original source is returned unchanged.

## Building from source

Requires: Rust (1.70+), [napi-rs CLI](https://napi.rs/docs/introduction/getting-started)

```sh
# Install napi-rs CLI
bun add -g @napi-rs/cli

# Debug build (for development)
bun run build:debug

# Release build
bun run build
```

The build produces `bun-js-beforeparse.<platform>.node` in the package root.

### Cross-compilation

napi-rs handles cross-compilation automatically in CI via the
[`@napi-rs/action`](https://github.com/napi-rs/package-template) GitHub Action. For
local cross-compilation, see the [napi-rs docs](https://napi.rs/docs/cross-build).

## Contributing

Issues and PRs welcome. The Rust source is in `src/lib.rs`; the TypeScript wrapper is
in `js/index.ts`.

## License

MIT
