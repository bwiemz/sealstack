import type { HttpClient } from "../http.js";

export class ReceiptsNamespace {
  constructor(private http: HttpClient) {}

  async get(id: string): Promise<unknown> {
    return this.http.request({
      method: "GET", path: `/v1/receipts/${encodeURIComponent(id)}`,
    });
  }
}
