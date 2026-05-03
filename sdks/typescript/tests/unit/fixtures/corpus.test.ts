// Parametrized fixture-driven test: iterates the contracts/fixtures/
// corpus and verifies every fixture round-trips through the SDK
// against an msw-mocked gateway. Per spec §12.3, this is the SDK
// side of the wire-shape contract; the Rust validator
// (`emit-fixtures`) covers the gateway side.
//
// Adding a new fixture requires registering its DISPATCH entry; an
// unmapped fixture fails loudly so the corpus and the test cannot
// silently drift.

import { describe, it, expect, beforeAll, afterAll, afterEach } from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse, type HttpHandler } from "msw";
import { readdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { SealStack, SealStackError } from "../../../src/index.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..", "..", "..", "..", "..", "contracts", "fixtures");

// msw + tough-cookie reject "test"/"example" public suffixes; use a
// non-public host. Matches the Python tests so both languages talk
// to the same fake.
const HOST = "http://localhost.sealstack.local";

interface RecordedReq {
  method: string;
  path: string;
  headers: Record<string, string>;
  body: unknown;
}

interface RecordedRes {
  status: number;
  headers: Record<string, string>;
  body: unknown;
}

type Dispatch = (c: SealStack, req: RecordedReq) => Promise<unknown>;

// Maps fixture name → SDK call that should reproduce the recorded
// request. A test for an unmapped fixture fails at registration
// time (in the loop below), not silently.
const DISPATCH: Record<string, Dispatch> = {
  "query-success": (c, r) => c.query({
    schema: (r.body as { schema: string }).schema,
    query: (r.body as { query: string }).query,
    topK: (r.body as { top_k?: number }).top_k,
  }),
  "query-policy-denied": (c, r) => c.query({
    schema: (r.body as { schema: string }).schema,
    query: (r.body as { query: string }).query,
  }),
  "register-schema-success": (c, r) => c.admin.schemas.register({
    meta: (r.body as { meta: unknown }).meta,
  }),
  "apply-ddl-success": (c, r) => c.admin.schemas.applyDdl(
    decodeURIComponent(r.path.split("/")[3]!),
    { ddl: (r.body as { ddl: string }).ddl },
  ),
  "apply-ddl-validation-error": (c, r) => c.admin.schemas.applyDdl(
    decodeURIComponent(r.path.split("/")[3]!),
    { ddl: (r.body as { ddl: string }).ddl },
  ),
  "register-connector-success": (c, r) => c.admin.connectors.register({
    kind: (r.body as { kind: string }).kind,
    schema: (r.body as { schema: string }).schema,
    config: (r.body as { config: unknown }).config,
  }),
  "sync-connector-success": (c, r) => c.admin.connectors.sync(
    decodeURIComponent(r.path.split("/")[3]!),
  ),
  "list-schemas-success": (c) => c.schemas.list(),
  "list-connectors-success": (c) => c.connectors.list(),
  "get-receipt-not-found": (c, r) => c.receipts.get(
    decodeURIComponent(r.path.split("/")[3]!),
  ),
  "healthz-success": (c) => c.healthz(),
};

const server = setupServer();
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

function handlerFor(method: string): (path: string, resolver: Parameters<typeof http.get>[1]) => HttpHandler {
  switch (method) {
    case "GET": return http.get;
    case "POST": return http.post;
    case "PUT": return http.put;
    case "DELETE": return http.delete;
    case "PATCH": return http.patch;
    default: throw new Error(`unsupported method: ${method}`);
  }
}

describe("fixture corpus", () => {
  const fixtures = readdirSync(ROOT, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();

  for (const name of fixtures) {
    it(`${name} round-trips through the SDK`, async () => {
      const dispatch = DISPATCH[name];
      if (!dispatch) {
        throw new Error(
          `no DISPATCH entry for fixture ${name}; add one in fixtures/corpus.test.ts`,
        );
      }

      const dir = join(ROOT, name);
      const req = JSON.parse(readFileSync(join(dir, "request.json"), "utf8")) as RecordedReq;
      const res = JSON.parse(readFileSync(join(dir, "response.json"), "utf8")) as RecordedRes;

      const url = `${HOST}${req.path}`;
      server.use(
        handlerFor(req.method)(url, async ({ request }) => {
          if (req.body !== null && req.body !== undefined && req.method !== "GET") {
            const body = await request.json();
            expect(body).toEqual(req.body);
          }
          return HttpResponse.json(res.body, { status: res.status, headers: res.headers });
        }),
      );

      const client = SealStack.bearer({ url: HOST, token: "test-token" });
      if (res.status >= 400) {
        await expect(dispatch(client, req)).rejects.toBeInstanceOf(SealStackError);
      } else {
        await dispatch(client, req);
      }
    });
  }
});
