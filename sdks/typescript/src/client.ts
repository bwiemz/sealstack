import { HttpClient, type HttpClientOptions } from "./http.js";
import { SchemasNamespace } from "./namespaces/schemas.js";
import { ConnectorsNamespace } from "./namespaces/connectors.js";
import { ReceiptsNamespace } from "./namespaces/receipts.js";
import { AdminNamespace } from "./namespaces/admin.js";

const LOCAL_HOSTS = ["localhost", "127.0.0.1", "host.docker.internal"];

function looksLikeLocal(url: string): boolean {
  try {
    const u = new URL(url);
    if (LOCAL_HOSTS.includes(u.hostname)) return true;
    if (u.hostname.endsWith(".local")) return true;
    return false;
  } catch {
    return false;
  }
}

export interface BearerOptions {
  url: string;
  token: string | (() => string);
  timeout?: number;
  retryAttempts?: number;
  retryInitialBackoffMs?: number;
  debug?: boolean | ((msg: string) => void);
}

export interface UnauthenticatedOptions {
  url: string;
  user: string;
  tenant: string;
  roles?: string[];
  timeout?: number;
  retryAttempts?: number;
  retryInitialBackoffMs?: number;
  debug?: boolean | ((msg: string) => void);
}

export class SealStack {
  readonly schemas: SchemasNamespace;
  readonly connectors: ConnectorsNamespace;
  readonly receipts: ReceiptsNamespace;
  readonly admin: AdminNamespace;
  // Hold a reference for top-level methods like query / healthz / readyz.
  // Private to keep the public surface tight.
  private readonly http: HttpClient;

  private constructor(http: HttpClient) {
    this.http = http;
    this.schemas = new SchemasNamespace(http);
    this.connectors = new ConnectorsNamespace(http);
    this.receipts = new ReceiptsNamespace(http);
    this.admin = new AdminNamespace(http);
  }

  static bearer(opts: BearerOptions): SealStack {
    const tokenFn = typeof opts.token === "function" ? opts.token : () => opts.token as string;
    const headers = (): Record<string, string> => ({ authorization: `Bearer ${tokenFn()}` });
    return new SealStack(makeHttp(opts, headers()));
  }

  static unauthenticated(opts: UnauthenticatedOptions): SealStack {
    if (!opts.tenant) {
      throw new TypeError("SealStack.unauthenticated() requires `tenant`");
    }
    if (!looksLikeLocal(opts.url)) {
      console.warn(
        `SealStack.unauthenticated() called against non-local URL ${opts.url}. ` +
        `Production gateways should reject these requests, but you should use ` +
        `bearer() in any code that runs outside your laptop.`
      );
    }
    const headers: Record<string, string> = {
      "x-sealstack-user": opts.user,
      "x-sealstack-tenant": opts.tenant,
    };
    if (opts.roles && opts.roles.length > 0) {
      headers["x-sealstack-roles"] = opts.roles.join(",");
    }
    return new SealStack(makeHttp(opts, headers));
  }

  /** POST /v1/query — returns the wire `QueryResponse` shape:
   *  `{ results: QueryHit[], receipt_id: string }`. */
  async query(req: { schema: string; query: string; topK?: number; filters?: unknown }): Promise<unknown> {
    return this.http.request({
      method: "POST",
      path: "/v1/query",
      body: { schema: req.schema, query: req.query, top_k: req.topK, filters: req.filters },
    });
  }

  async healthz(): Promise<unknown> {
    return this.http.request({ method: "GET", path: "/healthz" });
  }
  async readyz(): Promise<unknown> {
    return this.http.request({ method: "GET", path: "/readyz" });
  }
}

function makeHttp(
  opts: BearerOptions | UnauthenticatedOptions,
  headers: Record<string, string>,
): HttpClient {
  const debug = opts.debug === true
    ? (m: string) => console.debug("[sealstack]", m)
    : typeof opts.debug === "function"
      ? opts.debug
      : process.env.SEALSTACK_SDK_DEBUG === "1"
        ? (m: string) => console.debug("[sealstack]", m)
        : undefined;

  const httpOpts: HttpClientOptions = {
    baseUrl: opts.url.replace(/\/$/, ""),
    headers,
    timeoutMs: (opts.timeout ?? 30) * 1000,
    retryAttempts: opts.retryAttempts ?? 2,
    retryInitialBackoffMs: opts.retryInitialBackoffMs ?? 200,
    debug,
  };
  return new HttpClient(httpOpts);
}
