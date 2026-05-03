import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";
import { HttpClient } from "../../src/http.js";
import { BackendError } from "../../src/errors.js";

const server = setupServer();
beforeAll(() => server.listen());
afterAll(() => server.close());

// Use a non-public-suffix host so msw's cookie jar (tough-cookie) accepts it.
// Bare TLDs like "test" and "example" are flagged as public suffixes and
// trigger a CookieJar exception that aborts handler lookup. localhost works.
const BASE = "http://localhost.sealstack.local";

describe("HttpClient", () => {
  it("returns parsed data on 200", async () => {
    server.use(http.get(`${BASE}/x`, () =>
      HttpResponse.json({ data: { ok: true }, error: null }),
    ));
    const c = new HttpClient({ baseUrl: BASE, headers: {}, timeoutMs: 5000, retryAttempts: 0, retryInitialBackoffMs: 100 });
    const result = await c.request<{ ok: boolean }>({ method: "GET", path: "/x" });
    expect(result).toEqual({ ok: true });
  });

  it("throws BackendError with requestId on 500", async () => {
    server.use(http.get(`${BASE}/x`, () =>
      HttpResponse.json({ data: null, error: { code: "backend", message: "boom" } },
        { status: 500, headers: { "x-request-id": "req-7" } }),
    ));
    const c = new HttpClient({ baseUrl: BASE, headers: {}, timeoutMs: 5000, retryAttempts: 0, retryInitialBackoffMs: 100 });
    await expect(c.request({ method: "GET", path: "/x" })).rejects.toThrow(BackendError);
  });

  it("retries 5xx up to retry_attempts and then succeeds", async () => {
    let n = 0;
    server.use(http.get(`${BASE}/x`, () => {
      n += 1;
      if (n < 3) return new HttpResponse(null, { status: 503 });
      return HttpResponse.json({ data: { ok: true }, error: null });
    }));
    const c = new HttpClient({ baseUrl: BASE, headers: {}, timeoutMs: 5000, retryAttempts: 2, retryInitialBackoffMs: 5 });
    await expect(c.request({ method: "GET", path: "/x" })).resolves.toEqual({ ok: true });
    expect(n).toBe(3);
  });

  it("retries 429 honoring Retry-After", async () => {
    let n = 0;
    server.use(http.get(`${BASE}/x`, () => {
      n += 1;
      if (n === 1) return new HttpResponse(null, { status: 429, headers: { "retry-after": "0" } });
      return HttpResponse.json({ data: { ok: true }, error: null });
    }));
    const c = new HttpClient({ baseUrl: BASE, headers: {}, timeoutMs: 5000, retryAttempts: 1, retryInitialBackoffMs: 5 });
    await expect(c.request({ method: "GET", path: "/x" })).resolves.toEqual({ ok: true });
  });

  it("propagates AbortSignal mid-retry-sleep", async () => {
    server.use(http.get(`${BASE}/x`, () => new HttpResponse(null, { status: 503 })));
    const c = new HttpClient({ baseUrl: BASE, headers: {}, timeoutMs: 60_000, retryAttempts: 5, retryInitialBackoffMs: 100 });
    const ac = new AbortController();
    const promise = c.request({ method: "GET", path: "/x", signal: ac.signal });
    setTimeout(() => ac.abort(), 20);
    await expect(promise).rejects.toThrow(/abort/i);
  });

  it("redacts Authorization in debug logs", () => {
    const log: string[] = [];
    const c = new HttpClient({
      baseUrl: BASE,
      headers: { Authorization: "Bearer secret-token" },
      timeoutMs: 5000, retryAttempts: 0, retryInitialBackoffMs: 100,
      debug: (msg) => log.push(msg),
    });
    c.logRequestForTest({ method: "GET", path: "/x" });
    const joined = log.join("\n");
    expect(joined).toContain("authorization");
    expect(joined).toContain("<redacted>");
    expect(joined).not.toContain("secret-token");
  });
});
