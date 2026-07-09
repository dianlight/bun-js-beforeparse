/**
 * bun-js-beforeparse
 *
 * Lets you write Bun bundler `onBeforeParse` transforms in plain TypeScript/JavaScript
 * without any per-project Rust code.
 *
 * Usage:
 * ```ts
 * import { jsBridge } from "bun-js-beforeparse";
 *
 * Bun.serve({
 *   plugins: [{
 *     name: "my-transform",
 *     setup(build) {
 *       build.onBeforeParse(
 *         { filter: /\.[jt]sx$/, namespace: "file" },
 *         jsBridge(async (source, path) => {
 *           // your transform here — pure TypeScript, no Rust
 *           return source.replace("foo", "bar");
 *         }),
 *       );
 *     },
 *   }],
 * });
 * ```
 *
 * HOW IT WORKS
 * ─────────────
 * jsBridge() registers your JS function as a ThreadsafeFunction (TSFN) inside
 * a compiled Rust native module. When Bun calls the onBeforeParse C hook from
 * a worker thread, the bridge posts the source+path to the JS main thread,
 * blocks the worker, waits for your function's result, and writes it back.
 *
 * IMPORTANT CONSTRAINT
 * ─────────────────────
 * Your transform function must not await anything that requires the JS event
 * loop to "turn over" (e.g. `await fetch(...)`, `await Bun.file(...).text()`).
 * CPU-only async work (e.g. Babel transforms via @code-inspector/core) is safe
 * because those Promises resolve through microtasks without yielding the event
 * loop. If you need I/O, use the synchronous form instead.
 */

import { createRequire } from "module";

// Load the compiled .node binary.
// napi-rs generates this load logic; for a local build we load the .node directly.
const _require = createRequire(import.meta.url);
// Try the local .node first (development), then the package root (installed).
let native: ReturnType<typeof _require>;
try {
  const p = process.platform === "win32" ? "win32" : process.platform;
  const a = process.arch;
  const suffix =
    p === "linux"
      ? `linux-${a}-gnu`
      : p === "darwin"
        ? `darwin-${a}`
        : p === "win32"
          ? `win32-${a}-msvc`
          : null;
  if (!suffix) throw new Error(`Unsupported platform: ${p}-${a}`);
  native = _require(`../bun-js-beforeparse.${suffix}.node`);
} catch {
  native = _require("bun-js-beforeparse");
}

/**
 * A transform function that receives source code and file path,
 * and returns the (possibly modified) source code.
 *
 * May be synchronous or async. CPU-only async is safe (Babel, SWC, Oxc, etc.).
 * Do NOT use event-loop-bound async (fetch, file I/O) — it will deadlock.
 */
export type TransformFn = (source: string, path: string) => string | Promise<string>;

/**
 * The object shape that `build.onBeforeParse(matcher, HERE)` expects.
 */
export interface NativePluginDescriptor {
  napiModule: unknown;
  symbol: string;
  external: unknown;
}

/**
 * Wraps a TypeScript/JavaScript transform function for use as a Bun
 * `onBeforeParse` native plugin.
 *
 * @param fn - Your transform: (source, path) => transformedSource
 * @returns A descriptor to pass as the second argument of `build.onBeforeParse()`
 *
 * @example
 * ```ts
 * build.onBeforeParse(
 *   { filter: /\.[jt]sx$/ },
 *   jsBridge(async (source, path) => transformCode({ content: source, ... })),
 * );
 * ```
 */
export function jsBridge(fn: TransformFn): NativePluginDescriptor {
  // napi-rs CalleeHandled TSFN prepends a null "error" arg (Node.js error-first convention):
  // the native hook calls callback(null, source, path). We wrap fn to skip that first null
  // so the user's TransformFn cleanly receives (source, path).
  const wrappedFn = (_err: null, source: string, path: string) => fn(source, path);

  // createBridge() registers the wrapper as a ThreadsafeFunction inside the native module
  // and returns an External pointer (opaque to JS). The extern "C" hook finds the TSFN
  // via this pointer when Bun calls it from a worker thread.
  const external = native.createBridge(wrappedFn);

  return {
    napiModule: native,
    symbol: "bun_js_bridge_dispatch",
    external,
  };
}

/**
 * Release the TSFN reference held by a bridge, allowing the process to exit.
 *
 * Call this after `Bun.build()` or `Bun.serve()` has completed all builds.
 * Without this call, the ThreadsafeFunction keeps the event loop alive indefinitely.
 *
 * @example
 * ```ts
 * const descriptor = jsBridge(myTransform);
 * await Bun.build({ ..., plugins: [{ setup(build) { build.onBeforeParse(filter, descriptor); } }] });
 * releaseBridge(descriptor);
 * ```
 */
export function releaseBridge(descriptor: NativePluginDescriptor): void {
  native.releaseBridge(descriptor.external);
}
