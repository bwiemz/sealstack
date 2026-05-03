import type { HttpClient } from "../http.js";

export class SchemasNamespace {
  constructor(private http: HttpClient) {}

  async list(): Promise<unknown[]> {
    const data = await this.http.request<{ schemas: unknown[] }>({
      method: "GET", path: "/v1/schemas",
    });
    return data.schemas;
  }

  async get(qualified: string): Promise<unknown> {
    return this.http.request({
      method: "GET", path: `/v1/schemas/${encodeURIComponent(qualified)}`,
    });
  }
}
