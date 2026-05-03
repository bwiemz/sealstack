import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { SealStack } from "../../../src/index.js";

const __dirname = dirname(fileURLToPath(import.meta.url));

const FIXTURE = "query-success";
const root = join(__dirname, "..", "..", "..", "..", "..", "contracts", "fixtures", FIXTURE);
const req = JSON.parse(readFileSync(join(root, "request.json"), "utf8"));
const res = JSON.parse(readFileSync(join(root, "response.json"), "utf8"));

// msw + tough-cookie reject "test"/"example" public suffixes; use a non-public host.
const HOST = "http://localhost.sealstack.local";

const server = setupServer();
beforeAll(() => server.listen());
afterAll(() => server.close());

describe(`fixture: ${FIXTURE}`, () => {
  it("SDK round-trips the recorded request/response", async () => {
    server.use(http.post(`${HOST}${req.path}`, async ({ request }) => {
      const body = await request.json();
      expect(body).toEqual(req.body);
      return HttpResponse.json(res.body, { status: res.status, headers: res.headers });
    }));

    const client = SealStack.bearer({ url: HOST, token: "test-token" });
    const out = await client.query({
      schema: req.body.schema,
      query: req.body.query,
      topK: req.body.top_k,
    });
    expect(out).toEqual(res.body.data);
  });
});
