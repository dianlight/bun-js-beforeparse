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
import { writeFileSync, mkdirSync, rmSync } from "fs";

const _require = createRequire(import.meta.url);

// ─── Load the native module ──────────────────────────────────────────────────

let native: Record<string, unknown>;
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

// ─── Helper: create a temp entry file and build with onBeforeParse ───────────

const TMP_DIR = "/tmp/bun-js-beforeparse-test";

function createTmpFile(name: string, content: string): string {
  mkdirSync(TMP_DIR, { recursive: true });
  const filePath = `${TMP_DIR}/${name}`;
  writeFileSync(filePath, content);
  return filePath;
}

async function buildWithBridge(
  entryContent: string,
  entryName: string,
  transform: (source: string, path: string) => string | Promise<string>,
  filter: RegExp = /\.ts$/,
) {
  const entryPath = createTmpFile(entryName, entryContent);
  const result = await Bun.build({
    entrypoints: [entryPath],
    target: "browser",
    plugins: [
      {
        name: "test-bridge",
        setup(build) {
          build.onBeforeParse(
            { filter, namespace: "file" },
            jsBridge(transform),
          );
        },
      },
    ],
  });
  return result;
}

// ─── Sync transform tests ────────────────────────────────────────────────────

describe("jsBridge sync transform", () => {
  test("transforms source through Bun.build and output contains result", async () => {
    const sentinel = "SYNC_TRANSFORM_ACTIVE";
    const result = await buildWithBridge(
      `export const x = 1;`,
      "sync-test.ts",
      (source) => `// ${sentinel}\n${source}`,
    );

    expect(result.success).toBe(true);
    expect(result.outputs.length).toBeGreaterThan(0);

    const bundle = await result.outputs[0].text();
    expect(bundle).toContain(sentinel);
  });

  test("receives correct source and path arguments", async () => {
    let receivedPath = "";
    const result = await buildWithBridge(
      `export const val = "hello";`,
      "path-test.ts",
      (source, path) => {
        receivedPath = path;
        return source;
      },
    );

    expect(result.success).toBe(true);
    expect(receivedPath).toContain("path-test.ts");
  });

  test("can modify source content (uppercase transform)", async () => {
    const result = await buildWithBridge(
      `export const msg = "lowercase";`,
      "case-test.ts",
      (source) => source.replace("lowercase", "UPPERCASE"),
    );

    expect(result.success).toBe(true);
    const bundle = await result.outputs[0].text();
    expect(bundle).toContain("UPPERCASE");
    expect(bundle).not.toContain("lowercase");
  });

  test("transform returning empty string does not corrupt output (native falls back to original)", async () => {
    const original = `export const keep = 42;`;
    const result = await buildWithBridge(
      original,
      "empty-test.ts",
      () => "",
    );

    expect(result.success).toBe(true);
    const bundle = await result.outputs[0].text();
    expect(bundle).toContain("keep");
  });
});

// ─── Async transform tests ───────────────────────────────────────────────────

describe("jsBridge async transform", () => {
  test("transforms source through Bun.build with async function", async () => {
    const sentinel = "ASYNC_TRANSFORM_ACTIVE";
    const result = await buildWithBridge(
      `export const x = 1;`,
      "async-test.ts",
      async (source) => {
        // Simulate CPU-only async work (microtask resolution — safe for bridge)
        await Promise.resolve();
        return `// ${sentinel}\n${source}`;
      },
    );

    expect(result.success).toBe(true);
    expect(result.outputs.length).toBeGreaterThan(0);

    const bundle = await result.outputs[0].text();
    expect(bundle).toContain(sentinel);
  });

  test("async transform receives correct source and path", async () => {
    let receivedSource = "";
    let receivedPath = "";
    const result = await buildWithBridge(
      `export const data = 99;`,
      "async-args-test.ts",
      async (source, path) => {
        receivedSource = source;
        receivedPath = path;
        await Promise.resolve();
        return source;
      },
    );

    expect(result.success).toBe(true);
    expect(receivedSource).toContain("data = 99");
    expect(receivedPath).toContain("async-args-test.ts");
  });

  test("async transform with chained microtasks resolves correctly", async () => {
    const result = await buildWithBridge(
      `export const chain = 1;`,
      "async-chain-test.ts",
      async (source) => {
        const a = await Promise.resolve("FIRST");
        const b = await Promise.resolve("SECOND");
        return `// ${a}_${b}\n${source}`;
      },
    );

    expect(result.success).toBe(true);
    const bundle = await result.outputs[0].text();
    expect(bundle).toContain("FIRST_SECOND");
  });
});

// ─── Cleanup ─────────────────────────────────────────────────────────────────

test("cleanup temp files", () => {
  try {
    rmSync(TMP_DIR, { recursive: true, force: true });
  } catch {
    // ignore
  }
  expect(true).toBe(true);
});
