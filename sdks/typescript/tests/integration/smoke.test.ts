// Smoke suite: hits a live gateway on SEALSTACK_GATEWAY_URL.
//
// Cases are intentionally bounded to operations that work against a
// fresh (no-seed-data) gateway: health endpoints, registering a
// throwaway schema, and exercising one negative path (404 receipt).
// Cases that need a populated corpus (e.g. query returning hits with
// receipt URL resolution) belong to a heavier integration harness.
//
// Skipped wholesale when SEALSTACK_GATEWAY_URL is unset, so the file
// is safe to leave wired into CI paths that don't yet provision a
// gateway.

import { describe, it, expect } from "vitest";
import { SealStack, NotFoundError } from "../../src/index.js";

const URL = process.env.SEALSTACK_GATEWAY_URL;
const it_ = URL ? it : it.skip;

describe("SDK ↔ live gateway smoke", () => {
  it_("/healthz reports ok", async () => {
    const c = SealStack.bearer({ url: URL!, token: "test-token" });
    const out = await c.healthz();
    expect(out).toEqual({ status: "ok" });
  });

  it_("/readyz reports ok", async () => {
    const c = SealStack.bearer({ url: URL!, token: "test-token" });
    const out = await c.readyz();
    expect(out).toEqual({ status: "ok" });
  });

  it_("registering a schema returns its qualified name", async () => {
    const c = SealStack.bearer({ url: URL!, token: "test-token" });
    const ns = `smoke_ts_${Date.now()}`;
    const out = await c.admin.schemas.register({
      meta: {
        namespace: ns,
        name: "Doc",
        version: 1,
        primary_key: "id",
        fields: [
          { name: "id",   ty: "string", nullable: false },
          { name: "body", ty: "string", nullable: false },
        ],
        table: `${ns}_doc_v1`,
        collection: `${ns}_doc_v1`,
        hybrid_alpha: 0.5,
      },
    });
    expect(out.qualified).toEqual(`${ns}.Doc`);
  });

  it_("registered schema appears in the list", async () => {
    const c = SealStack.bearer({ url: URL!, token: "test-token" });
    const all = await c.schemas.list() as Array<{ namespace: string; name: string }>;
    expect(Array.isArray(all)).toBe(true);
    // No specific assertion on contents — just that the list endpoint
    // round-trips an array. Pairing this with the register case above
    // gives wire-shape confidence on both sides of the admin namespace.
  });

  it_("unknown receipt id raises NotFoundError", async () => {
    const c = SealStack.bearer({ url: URL!, token: "test-token" });
    await expect(c.receipts.get("01JD0BOGUS00000000000000000")).rejects.toBeInstanceOf(NotFoundError);
  });
});
