import type { HttpClient } from "../http.js";

class AdminSchemasNamespace {
  constructor(private http: HttpClient) {}

  async register(req: { meta: unknown }): Promise<{ qualified: string }> {
    return this.http.request({
      method: "POST", path: "/v1/schemas",
      body: req, noRetry: true,
    });
  }

  async applyDdl(qualified: string, req: { ddl: string }): Promise<{ applied: number }> {
    return this.http.request({
      method: "POST", path: `/v1/schemas/${encodeURIComponent(qualified)}/ddl`,
      body: req, noRetry: true, timeoutMs: 60_000,
    });
  }
}

class AdminConnectorsNamespace {
  constructor(private http: HttpClient) {}

  async register(req: { kind: string; schema: string; config: unknown }): Promise<{ id: string }> {
    return this.http.request({
      method: "POST", path: "/v1/connectors",
      body: req, noRetry: true,
    });
  }

  async sync(id: string): Promise<{ jobId: string }> {
    const out = await this.http.request<{ job_id: string }>({
      method: "POST", path: `/v1/connectors/${encodeURIComponent(id)}/sync`,
      noRetry: true,
    });
    return { jobId: out.job_id };
  }
}

export class AdminNamespace {
  readonly schemas: AdminSchemasNamespace;
  readonly connectors: AdminConnectorsNamespace;

  constructor(http: HttpClient) {
    this.schemas = new AdminSchemasNamespace(http);
    this.connectors = new AdminConnectorsNamespace(http);
  }
}
