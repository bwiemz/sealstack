import type { HttpClient } from "../http.js";

export class ConnectorsNamespace {
  constructor(private http: HttpClient) {}

  async list(): Promise<unknown[]> {
    const data = await this.http.request<{ connectors: unknown[] }>({
      method: "GET", path: "/v1/connectors",
    });
    return data.connectors;
  }
}
