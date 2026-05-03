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

pub mod envelope;
pub mod query;
pub mod schemas;
pub mod connectors;
pub mod receipts;
pub mod health;

pub use envelope::{Envelope, ErrorDetail, ErrorCode};
