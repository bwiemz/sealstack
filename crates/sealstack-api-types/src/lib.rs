//! SealStack gateway wire types.
//!
//! These structs define the JSON shapes the gateway accepts and emits on
//! its REST surface. They derive `JsonSchema` so the `emit-schemas` binary
//! can produce JSON Schema artifacts that drive the TypeScript and Python
//! SDK codegen pipelines.
//!
//! See `../../../docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md`
//! for the URL-and-semantics layer that the JSON Schemas do not cover.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod connectors;
pub mod envelope;
pub mod health;
pub mod query;
pub mod receipts;
pub mod schemas;

pub use envelope::{Envelope, ErrorCode, ErrorDetail};
