import { describe, it, expect } from "vitest";
import {
  SealStackError, NotFoundError, UnknownSchemaError,
  UnauthorizedError, PolicyDeniedError, InvalidArgumentError,
  RateLimitedError, BackendError, fromWireError,
} from "../../src/errors.js";

describe("error hierarchy", () => {
  it("NotFoundError extends SealStackError", () => {
    const e = new NotFoundError("missing", "schema:Foo");
    expect(e).toBeInstanceOf(SealStackError);
    expect(e.name).toBe("NotFoundError");
    expect(e.resource).toBe("schema:Foo");
  });

  it("UnknownSchemaError extends NotFoundError", () => {
    const e = new UnknownSchemaError("no such schema", "examples.Foo");
    expect(e).toBeInstanceOf(NotFoundError);
    expect(e).toBeInstanceOf(SealStackError);
    expect(e.schema).toBe("examples.Foo");
  });

  it("PolicyDeniedError carries predicate", () => {
    const e = new PolicyDeniedError("denied", "rule.admin_only");
    expect(e.predicate).toBe("rule.admin_only");
  });

  it("RateLimitedError.retry_after is optional", () => {
    expect(new RateLimitedError("slow down", null).retryAfter).toBeNull();
    expect(new RateLimitedError("slow down", 60).retryAfter).toBe(60);
  });

  it("BackendError.requestId is required", () => {
    const e = new BackendError("kaboom", "req-abc");
    expect(e.requestId).toBe("req-abc");
  });

  it.each([
    ["not_found", "Doc", NotFoundError],
    ["unknown_schema", "Doc", UnknownSchemaError],
    ["unauthorized", "msg", UnauthorizedError],
    ["policy_denied", "rule", PolicyDeniedError],
    ["invalid_argument", "field 'x' missing", InvalidArgumentError],
    ["rate_limited", "slow down", RateLimitedError],
    ["backend", "kaboom", BackendError],
  ])("fromWireError dispatches %s -> right class", (code, message, klass) => {
    const e = fromWireError(
      { code: code as never, message },
      { headers: { "x-request-id": "req-1", "retry-after": "30" } },
    );
    expect(e).toBeInstanceOf(klass);
  });

  it("fromWireError falls back to BackendError on unknown code", () => {
    const e = fromWireError(
      { code: "made_up_code" as never, message: "unknown" },
      { headers: { "x-request-id": "req-1" } },
    );
    expect(e).toBeInstanceOf(BackendError);
    expect(e.message).toContain("made_up_code");
  });
});
