//! Authentication primitives for connectors.
//!
//! v1 ships `StaticToken` (PATs, bot tokens, API keys). Future modules add
//! OAuth 2.0 authorization-code + refresh, Google service-account JWTs, etc.,
//! each as an additional [`Credential`] implementation.

// Trait + impl land in Task 2.
