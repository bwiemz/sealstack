import { defineConfig } from "vitest/config";

// Integration smoke suite — runs against a live gateway on
// process.env.SEALSTACK_GATEWAY_URL. The unit suite (default
// `vitest run`) excludes tests/integration/, so the two are
// orthogonal: `test` runs unit tests with msw mocks; `test:integration`
// runs smoke tests against the real gateway.
export default defineConfig({
  test: {
    include: ["tests/integration/**/*.test.ts"],
    testTimeout: 20_000,
  },
});
