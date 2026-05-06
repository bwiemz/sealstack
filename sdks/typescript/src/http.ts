import { fromWireError, SealStackError, BackendError } from "./errors.js";

/** Headers redacted from debug logs (case-insensitive). */
export const REDACTED_HEADERS = new Set([
  "authorization",
  "cookie",
  "x-api-key",
  "x-sealstack-user",
  "x-sealstack-tenant",
  "x-sealstack-roles",
  "x-cfg-user",
  "x-cfg-tenant",
  "x-cfg-roles",
]);

/**
 * Per-request headers. Static dict for the common case; a factory `() => dict`
 * for token-rotating clients (`SealStack.bearer({ token: () => fetch_jwt() })`),
 * which would otherwise bind to a single token at client construction.
 */
export type HeadersSource = Record<string, string> | (() => Record<string, string>);

export interface HttpClientOptions {
  baseUrl: string;
  headers: HeadersSource;
  timeoutMs: number;
  retryAttempts: number;
  retryInitialBackoffMs: number;
  debug?: (msg: string) => void;
}

export interface RequestOptions {
  method: "GET" | "POST";
  path: string;
  body?: unknown;
  signal?: AbortSignal;
  /** Per-call override of the client default. */
  timeoutMs?: number;
  /** Skip retry policy (admin namespace uses this). */
  noRetry?: boolean;
}

interface Envelope<T> {
  data: T | null;
  error: { code: string; message: string } | null;
}

export class HttpClient {
  constructor(private opts: HttpClientOptions) {}

  private resolveHeaders(): Record<string, string> {
    return typeof this.opts.headers === "function" ? this.opts.headers() : this.opts.headers;
  }

  /** Public test hook for the redaction logic; not for production callers. */
  logRequestForTest(req: { method: string; path: string }): void {
    this.logRequest(req, this.resolveHeaders());
  }

  async request<T>(req: RequestOptions): Promise<T> {
    const maxAttempts = req.noRetry ? 1 : this.opts.retryAttempts + 1;
    const timeoutMs = req.timeoutMs ?? this.opts.timeoutMs;
    let lastError: unknown;

    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      try {
        return await this.attempt<T>(req, timeoutMs);
      } catch (e) {
        lastError = e;
        if (req.signal?.aborted) throw e;
        if (!this.shouldRetry(e, attempt, maxAttempts)) throw e;
        await this.sleepWithCancel(this.backoffMs(attempt, e), req.signal);
      }
    }
    throw lastError;
  }

  private async attempt<T>(req: RequestOptions, timeoutMs: number): Promise<T> {
    // Resolve headers exactly once per attempt: a token-rotation factory
    // backed by a remote JWT fetch must not fire twice (once for the log,
    // once for the request) and the log must show the same token that
    // actually went out on the wire.
    const reqHeaders = this.resolveHeaders();
    this.logRequest(req, reqHeaders);
    const url = `${this.opts.baseUrl}${req.path}`;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    if (req.signal) req.signal.addEventListener("abort", () => controller.abort(), { once: true });

    let res: Response;
    try {
      res = await fetch(url, {
        method: req.method,
        headers: { "content-type": "application/json", ...reqHeaders },
        body: req.body == null ? undefined : JSON.stringify(req.body),
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timer);
    }

    const headers = headersToObject(res.headers);
    const text = await res.text();
    const env: Envelope<T> = text ? JSON.parse(text) : { data: null, error: null };

    if (res.status >= 400 || env.error) {
      this.logErrorResponse(res.status, headers, text);
      const wireErr = env.error ?? { code: "backend", message: `HTTP ${res.status}` };
      throw fromWireError(wireErr, { headers });
    }
    if (env.data == null) {
      throw new BackendError("response envelope missing data", headers["x-request-id"] ?? "unknown");
    }
    return env.data;
  }

  private shouldRetry(e: unknown, attempt: number, max: number): boolean {
    if (attempt >= max) return false;
    if (e instanceof SealStackError) {
      // Retriable: rate_limited (429) and backend (5xx). Per spec §9.1.
      return e.constructor.name === "RateLimitedError"
          || e.constructor.name === "BackendError";
    }
    // Network/abort errors retry.
    return e instanceof TypeError; // fetch network failure
  }

  private backoffMs(attempt: number, e: unknown): number {
    // Honor Retry-After on RateLimitedError if present.
    if (e instanceof SealStackError && e.constructor.name === "RateLimitedError") {
      const ra = (e as { retryAfter?: number }).retryAfter;
      if (ra != null && ra >= 0) return ra * 1000;
    }
    const base = this.opts.retryInitialBackoffMs * 2 ** (attempt - 1);
    // Full jitter: uniform random in [0, base * 1.25].
    return Math.random() * base * 1.25;
  }

  private async sleepWithCancel(ms: number, signal: AbortSignal | undefined): Promise<void> {
    return new Promise((resolve, reject) => {
      const t = setTimeout(resolve, ms);
      signal?.addEventListener("abort", () => {
        clearTimeout(t);
        reject(new Error("aborted"));
      }, { once: true });
    });
  }

  private logRequest(
    req: { method: string; path: string },
    headers: Record<string, string>,
  ): void {
    if (!this.opts.debug) return;
    this.opts.debug(`→ ${req.method} ${req.path} headers=${JSON.stringify(redactHeaders(headers))}`);
  }

  private logErrorResponse(status: number, headers: Record<string, string>, body: string): void {
    if (!this.opts.debug) return;
    this.opts.debug(`← ${status} headers=${JSON.stringify(redactHeaders(headers))} body=${body}`);
  }
}

function redactHeaders(h: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(h)) {
    out[k.toLowerCase()] = REDACTED_HEADERS.has(k.toLowerCase()) ? "<redacted>" : v;
  }
  return out;
}

function headersToObject(h: Headers): Record<string, string> {
  const out: Record<string, string> = {};
  h.forEach((v, k) => { out[k.toLowerCase()] = v; });
  return out;
}
