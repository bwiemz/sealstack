export interface SignetClientOptions {
  baseUrl: string;
  token?: string;
}

export interface ContextQuery {
  query: string;
  topK?: number;
  filters?: Record<string, unknown>;
}

export interface ContextChunk {
  id: string;
  content: string;
  score: number;
  metadata: Record<string, unknown>;
}

export interface Receipt {
  id: string;
  sources: Array<{ chunkId: string; sourceUri: string; score: number }>;
}

export interface ContextResponse {
  chunks: ContextChunk[];
  receipt: Receipt;
}

export interface Schema {
  name: string;
  version: number;
  namespace?: string;
}

export interface Connector {
  id: string;
  name: string;
  enabled: boolean;
}

export class SignetClient {
  readonly baseUrl: string;
  readonly token?: string;
  readonly schemas: SchemasApi;
  readonly connectors: ConnectorsApi;

  constructor(opts: SignetClientOptions) {
    this.baseUrl = opts.baseUrl.replace(/\/$/, "");
    this.token = opts.token;
    this.schemas = new SchemasApi(this);
    this.connectors = new ConnectorsApi(this);
  }

  async query(q: string, opts: Omit<ContextQuery, "query"> = {}): Promise<ContextResponse> {
    return this.request("POST", "/v1/query", { query: q, ...opts }) as Promise<ContextResponse>;
  }

  async request(method: string, path: string, body?: unknown): Promise<unknown> {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (this.token) headers.authorization = `Bearer ${this.token}`;
    const res = await fetch(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`signet: ${res.status} ${res.statusText}`);
    return res.json();
  }
}

class SchemasApi {
  constructor(private client: SignetClient) {}
  list(): Promise<Schema[]> {
    return this.client.request("GET", "/v1/schemas") as Promise<Schema[]>;
  }
  get(name: string): Promise<Schema> {
    return this.client.request("GET", `/v1/schemas/${encodeURIComponent(name)}`) as Promise<Schema>;
  }
}

class ConnectorsApi {
  constructor(private client: SignetClient) {}
  list(): Promise<Connector[]> {
    return this.client.request("GET", "/v1/connectors") as Promise<Connector[]>;
  }
  sync(id: string): Promise<{ jobId: string }> {
    return this.client.request(
      "POST",
      `/v1/connectors/${encodeURIComponent(id)}/sync`,
    ) as Promise<{ jobId: string }>;
  }
}
