// TypeScript mirrors of the engine's public DTOs.
//
// These shapes must stay in sync with:
//   signet_engine::api::{SearchRequest, SearchResponse, Caller}
//   signet_engine::schema_registry::SchemaMeta
//   signet_engine::receipts::Receipt
//   signet_ingest::registry::ConnectorBindingInfo
//   signet_ingest::runtime::SyncOutcome
//
// We keep them narrow — console never sends anything the CLI can't, and the
// CLI is the reference implementation.

// --- Envelope ---------------------------------------------------------------

export type Envelope<T> =
  | { data: T; error: null }
  | { data: null; error: { code: string; message: string } };

// --- Health ----------------------------------------------------------------

export interface Healthz {
  status: 'ok';
}

// --- Schemas ---------------------------------------------------------------

export interface SchemaSummary {
  namespace: string;
  name: string;
  version: number;
  table?: string;
  facets?: string[];
  relations?: string[];
}

export interface FieldMeta {
  name: string;
  column: string;
  ty: string;
  primary: boolean;
  indexed: boolean;
  searchable: boolean;
  chunked: boolean;
  facet: boolean;
  optional: boolean;
  unique: boolean;
  boost: number | null;
  pii: string | null;
}

export interface RelationMeta {
  name: string;
  kind: 'one' | 'many' | string;
  target_namespace: string;
  target_schema: string;
  foreign_key: string | null;
}

export interface ContextMeta {
  embedder: string;
  vector_dims: number;
  chunking: Record<string, unknown>;
  freshness_decay: Record<string, unknown>;
  default_top_k: number | null;
}

export interface SchemaMeta {
  namespace: string;
  name: string;
  version: number;
  primary_key: string;
  fields: FieldMeta[];
  relations: Record<string, RelationMeta>;
  facets: string[];
  chunked_fields: string[];
  context: ContextMeta;
  collection: string;
  table: string;
  hybrid_alpha: number | null;
}

// --- Connectors ------------------------------------------------------------

export interface ConnectorBinding {
  id: string;
  connector: string;
  version: string;
  namespace: string;
  schema: string;
  interval_secs: number | null;
}

export type SyncOutcomeKind = 'completed' | 'failed_list' | 'cancelled' | 'not_found';

export interface SyncOutcome {
  binding_id: string;
  kind: SyncOutcomeKind;
  resources_seen: number;
  resources_ingested: number;
  resources_failed: number;
  chunks_written: number;
  started_at: string;
  finished_at: string;
  elapsed_ms: number;
  error?: string | null;
}

// --- Query / results ------------------------------------------------------

export interface QueryRequest {
  schema: string;
  query: string;
  top_k?: number;
  filters?: Record<string, unknown>;
}

export interface SearchHit {
  id: string;
  score: number;
  excerpt: string;
  metadata?: Record<string, unknown>;
  chunk_id?: string;
  record_id?: string;
}

export interface SearchResponse {
  results: SearchHit[];
  receipt_id: string;
  elapsed_ms: number;
  total_candidates?: number;
}

// --- Receipts -------------------------------------------------------------

export interface ReceiptSource {
  record_id: string;
  chunk_id?: string;
  score: number;
  excerpt: string;
  freshness_boost?: number;
  rerank_score?: number;
}

export interface ReceiptPolicyVerdict {
  rule: string;
  decision: 'allow' | 'deny';
  message?: string;
}

export interface ReceiptTimings {
  embed_ms: number;
  vector_ms: number;
  bm25_ms: number;
  fuse_ms: number;
  rerank_ms: number;
  policy_ms: number;
  total_ms: number;
}

export interface Receipt {
  id: string;
  issued_at: string;
  caller: {
    id: string;
    tenant: string;
    roles: string[];
  };
  schema: {
    namespace: string;
    name: string;
    version: number;
  };
  query: string;
  filters?: Record<string, unknown>;
  sources: ReceiptSource[];
  verdicts: ReceiptPolicyVerdict[];
  timings: ReceiptTimings;
  signature?: string;
}
