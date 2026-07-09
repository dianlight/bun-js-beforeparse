/**
 * Integration test: verifies that jsBridge() actually fires in Bun.build()
 * and the JS transform is called + its result appears in the bundle.
 *
 * This is the key proof-of-concept: onBeforeParse fires for .tsx files
 * (which JS onLoad cannot do in Bun 1.3.x).
 */

import { jsBridge } from "../js/index.ts";
import { writeFileSync, mkdirSync } from "fs";

// Write a minimal test .tsx file
mkdirSync("/tmp/bridge-test-src", { recursive: true });
writeFileSync("/tmp/bridge-test-src/app.tsx", `
export function Hello() {
  return <div>Hello World</div>;
}
`);

console.log("Testing jsBridge() in Bun.build()...\n");

let callCount = 0;

const result = await Bun.build({
  entrypoints: ["/tmp/bridge-test-src/app.tsx"],
  target: "browser",
  plugins: [
    {
      name: "js-bridge-test",
      setup(build) {
        // This is the line that proves the NAPI bridge works:
        // onBeforeParse fires for .tsx (impossible with plain JS onLoad).
        build.onBeforeParse(
          { filter: /\.tsx$/, namespace: "file" },
          jsBridge((source: string, path: string) => {
            callCount++;
            console.log(`  [bridge] transform called for: ${path}`);
            // Inject a sentinel comment we can detect in the bundle
            return `// BRIDGE_TRANSFORM_APPLIED\n${source}`;
          }),
        );
      },
    },
  ],
});

console.log(`Build success: ${result.success}`);
console.log(`Transform call count: ${callCount}`);

if (result.success && result.outputs.length > 0) {
  const bundle = await result.outputs[0].text();
  const bridgeApplied = bundle.includes("BRIDGE_TRANSFORM_APPLIED");
  console.log(`Sentinel in bundle: ${bridgeApplied}`);

  if (callCount > 0 && bridgeApplied) {
    console.log("\n✅ SUCCESS: onBeforeParse bridge works! JS transform was called for .tsx.\n");
    process.exit(0);
  } else if (callCount === 0) {
    console.log("\n❌ FAIL: transform was never called (onBeforeParse did not fire).\n");
    process.exit(1);
  } else {
    console.log("\n❌ FAIL: transform was called but sentinel not in output.\n");
    process.exit(1);
  }
} else {
  console.log("Build logs:", result.logs.map(l => l.message));
  process.exit(1);
}
