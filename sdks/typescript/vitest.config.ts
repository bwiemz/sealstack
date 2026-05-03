import { defineConfig } from "vitest/config";

// Unit-only run. tests/integration/ is exclusively driven by
// vitest.integration.config.ts so msw-mocked unit tests and live-gateway
// smoke tests don't accidentally co-mingle in CI.
export default defineConfig({
  test: {
    include: ["tests/unit/**/*.test.ts"],
  },
});
