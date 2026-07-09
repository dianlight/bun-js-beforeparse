/**
 * Smoke tests for bun-js-beforeparse.
 *
 * Tests what can be verified without a full Bun.build() invocation:
 * - The .node module loads and exports createBridge
 * - createBridge returns an object with the right shape
 * - jsBridge() returns the correct descriptor
 *
 * Full integration (Bun.build + onBeforeParse) requires the Bun bundler
 * to actually call the native hook — tested manually via the POC.
 */

import { describe, test, expect } from "bun:test";
import { createRequire } from "module";

const _require = createRequire(import.meta.url);

// ─── Load the native module ──────────────────────────────────────────────────

let native: Record<string, unknown>;
try {
  native = _require("../bun-js-beforeparse.linux-x64-gnu.node");
} catch {
  native = _require("../bun-js-beforeparse.darwin-arm64.node");
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("native module", () => {
  test("loads successfully", () => {
    expect(native).toBeDefined();
  });

  test("exports createBridge function", () => {
    expect(typeof native.createBridge).toBe("function");
  });

  test("createBridge accepts a callback and returns an External", () => {
    const external = (native.createBridge as Function)((s: string) => s);
    // External is an opaque object in napi-rs v2; just verify it's not null/undefined
    expect(external).not.toBeNull();
    expect(external).not.toBeUndefined();
  });
});

// Import at module level (top-level await works in ESM / Bun test runner)
import { jsBridge } from "../js/index.ts";

describe("jsBridge() TypeScript wrapper", () => {

  test("returns a descriptor with napiModule, symbol, external", () => {
    const descriptor = jsBridge((s: string) => s.toUpperCase());
    expect(descriptor).toHaveProperty("napiModule");
    expect(descriptor).toHaveProperty("symbol", "bun_js_bridge_dispatch");
    expect(descriptor).toHaveProperty("external");
    expect(descriptor.external).not.toBeNull();
  });

  test("accepts an async transform function", () => {
    const descriptor = jsBridge(async (source: string, path: string) => {
      return source + "// transformed by async fn\n";
    });
    expect(descriptor.symbol).toBe("bun_js_bridge_dispatch");
  });
});
