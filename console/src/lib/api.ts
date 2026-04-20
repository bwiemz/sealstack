// Gateway HTTP client.
//
// Mirrors the CLI's `src/client.rs`: every call unwraps the `{ data, error }`
// envelope and throws a typed `ApiError` on gateway errors. Network failures
// surface as plain `Error`.

import type {
  ConnectorBinding,
  Envelope,
  Receipt,
  SchemaMeta,
  SchemaSummary,
  SearchResponse,
  SyncOutcome
} from './types';

export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status?: number
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export interface ClientOptions {
  gatewayUrl?: string;
  user?: string;
  fetch?: typeof fetch;
}

export class Client {
  private readonly base: string;
  private readonly user: string;
  private readonly fetchImpl: typeof fetch;

  constructor(opts: ClientOptions = {}) {
    const raw =
      opts.gatewayUrl ??
      (typeof window !== 'undefined' && (window as any).__SIGNET_GATEWAY_URL__) ??
      'http://localhost:7070';
    this.base = raw.replace(/\/+$/, '');
    this.user = opts.user ?? 'console';
    this.fetchImpl = opts.fetch ?? fetch;
  }

  // --- Raw ----------------------------------------------------------------

  private async req<T>(
    method: 'GET' | 'POST',
    path: string,
    body?: unknown
  ): Promise<T> {
    const headers: Record<string, string> = {
      Accept: 'application/json',
      'X-Cfg-User': this.user
    };
    if (body !== undefined) headers['Content-Type'] = 'application/json';

    const resp = await this.fetchImpl(`${this.base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body)
    });

    let parsed: Envelope<T>;
    try {
      parsed = (await resp.json()) as Envelope<T>;
    } catch {
      throw new ApiError('bad_response', `non-JSON body from ${path}`, resp.status);
    }

    if (parsed.error) {
      throw new ApiError(parsed.error.code, parsed.error.message, resp.status);
    }
    return parsed.data as T;
  }

  // --- Endpoints ----------------------------------------------------------

  async healthz(): Promise<boolean> {
    try {
      const resp = await this.fetchImpl(`${this.base}/healthz`, { method: 'GET' });
      return resp.ok;
    } catch {
      return false;
    }
  }

  async listSchemas(): Promise<SchemaSummary[]> {
    const data = await this.req<{ schemas: SchemaSummary[] }>('GET', '/v1/schemas');
    return data.schemas ?? [];
  }

  async getSchema(qualified: string): Promise<SchemaMeta> {
    return this.req<SchemaMeta>('GET', `/v1/schemas/${encodeURIComponent(qualified)}`);
  }

  async listConnectors(): Promise<ConnectorBinding[]> {
    const data = await this.req<{ connectors: ConnectorBinding[] }>(
      'GET',
      '/v1/connectors'
    );
    return data.connectors ?? [];
  }

  async registerConnector(input: {
    kind: string;
    schema: string;
    config: Record<string, unknown>;
  }): Promise<{ id: string; status: string }> {
    return this.req('POST', '/v1/connectors', input);
  }

  async syncConnector(id: string): Promise<SyncOutcome> {
    return this.req('POST', `/v1/connectors/${encodeURIComponent(id)}/sync`, {});
  }

  async query(input: {
    schema: string;
    query: string;
    top_k?: number;
    filters?: Record<string, unknown>;
  }): Promise<SearchResponse> {
    return this.req('POST', '/v1/query', input);
  }

  async getReceipt(id: string): Promise<Receipt> {
    return this.req('GET', `/v1/receipts/${encodeURIComponent(id)}`);
  }
}

/* Default singleton for client-side code. Routes that need to override for
   server-side rendering construct their own `Client` via `new Client({ fetch })`. */
export const client = new Client();
