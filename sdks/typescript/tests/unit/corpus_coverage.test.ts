import { describe, it, expect } from "vitest";
import { readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

/** Fixtures consumed by the TS SDK's tests. Update this list as you add
 *  tests that exercise new fixtures; the assertion at the bottom will
 *  fail if a fixture is added to contracts/fixtures/ without being
 *  consumed here. */
export const TS_CONSUMED_FIXTURES = new Set<string>([
  "query-success",
]);

describe("corpus coverage", () => {
  it("every fixture in contracts/fixtures/ is consumed by the TS SDK", () => {
    const root = join(__dirname, "..", "..", "..", "..", "contracts", "fixtures");
    const all = readdirSync(root, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name);
    const missing = all.filter((name) => !TS_CONSUMED_FIXTURES.has(name));
    expect(missing).toEqual([]);
  });
});
