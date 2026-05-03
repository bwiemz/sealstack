/** Base class for every typed SDK error. All subclasses extend Error. */
export class SealStackError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SealStackError";
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class NotFoundError extends SealStackError {
  readonly resource: string;
  constructor(message: string, resource: string) {
    super(message);
    this.name = "NotFoundError";
    this.resource = resource;
    Object.setPrototypeOf(this, NotFoundError.prototype);
  }
}

export class UnknownSchemaError extends NotFoundError {
  readonly schema: string;
  constructor(message: string, schema: string) {
    super(message, `schema:${schema}`);
    this.name = "UnknownSchemaError";
    this.schema = schema;
    Object.setPrototypeOf(this, UnknownSchemaError.prototype);
  }
}

export class UnauthorizedError extends SealStackError {
  readonly realm: string | null;
  constructor(message: string, realm: string | null = null) {
    super(message);
    this.name = "UnauthorizedError";
    this.realm = realm;
    Object.setPrototypeOf(this, UnauthorizedError.prototype);
  }
}

export class PolicyDeniedError extends SealStackError {
  readonly predicate: string;
  constructor(message: string, predicate: string) {
    super(message);
    this.name = "PolicyDeniedError";
    this.predicate = predicate;
    Object.setPrototypeOf(this, PolicyDeniedError.prototype);
  }
}

export class InvalidArgumentError extends SealStackError {
  readonly field: string | null;
  readonly reason: string;
  constructor(message: string, reason: string, field: string | null = null) {
    super(message);
    this.name = "InvalidArgumentError";
    this.field = field;
    this.reason = reason;
    Object.setPrototypeOf(this, InvalidArgumentError.prototype);
  }
}

export class RateLimitedError extends SealStackError {
  readonly retryAfter: number | null;
  constructor(message: string, retryAfter: number | null) {
    super(message);
    this.name = "RateLimitedError";
    this.retryAfter = retryAfter;
    Object.setPrototypeOf(this, RateLimitedError.prototype);
  }
}

export class BackendError extends SealStackError {
  readonly requestId: string;
  constructor(message: string, requestId: string) {
    super(message);
    this.name = "BackendError";
    this.requestId = requestId;
    Object.setPrototypeOf(this, BackendError.prototype);
  }
}

interface WireError {
  code: string;
  message: string;
}

interface ErrorContext {
  headers: Record<string, string>;
}

/** Dispatch a wire error envelope to the right typed class.
 *  Unknown codes fall through to BackendError per spec §8.2. */
export function fromWireError(wire: WireError, ctx: ErrorContext): SealStackError {
  const reqId = ctx.headers["x-request-id"] ?? "unknown";
  const retryAfter = ctx.headers["retry-after"]
    ? Number.parseInt(ctx.headers["retry-after"], 10)
    : null;

  switch (wire.code) {
    case "not_found":
      return new NotFoundError(wire.message, "<unspecified>");
    case "unknown_schema":
      return new UnknownSchemaError(wire.message, "<unspecified>");
    case "unauthorized":
      return new UnauthorizedError(wire.message);
    case "policy_denied":
      return new PolicyDeniedError(wire.message, "<unspecified>");
    case "invalid_argument":
      return new InvalidArgumentError(wire.message, wire.message);
    case "rate_limited":
      return new RateLimitedError(wire.message, retryAfter);
    case "backend":
      return new BackendError(wire.message, reqId);
    default:
      return new BackendError(`unknown error code: ${wire.code} (${wire.message})`, reqId);
  }
}
