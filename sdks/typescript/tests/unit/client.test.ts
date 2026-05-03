import { describe, it, expect, vi } from "vitest";
import { SealStack } from "../../src/index.js";

describe("SealStack factories", () => {
  it("bearer factory accepts a string token", () => {
    const c = SealStack.bearer({ url: "http://localhost:7070", token: "abc" });
    expect(c).toBeDefined();
  });

  it("bearer factory accepts a callable token", () => {
    const c = SealStack.bearer({ url: "http://localhost:7070", token: () => "abc" });
    expect(c).toBeDefined();
  });

  it("unauthenticated factory requires tenant", () => {
    expect(() =>
      // @ts-expect-error - missing tenant
      SealStack.unauthenticated({ url: "http://localhost:7070", user: "alice" })
    ).toThrow(TypeError);
  });

  it("unauthenticated emits warning for non-local URL", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    SealStack.unauthenticated({
      url: "https://gateway.acme.com",
      user: "alice", tenant: "default",
    });
    expect(warn).toHaveBeenCalled();
    expect(warn.mock.calls[0]?.[0]).toMatch(/non-local/i);
    warn.mockRestore();
  });

  it("unauthenticated does NOT warn for localhost", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    SealStack.unauthenticated({
      url: "http://localhost:7070",
      user: "alice", tenant: "default",
    });
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it("exposes read namespaces flat and admin under .admin", () => {
    const c = SealStack.bearer({ url: "http://localhost:7070", token: "abc" });
    expect(c.schemas).toBeDefined();
    expect(c.connectors).toBeDefined();
    expect(c.receipts).toBeDefined();
    expect(c.admin).toBeDefined();
    expect(c.admin.schemas).toBeDefined();
    expect(c.admin.connectors).toBeDefined();
  });
});
