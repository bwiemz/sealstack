//! Typed identity primitive for resource permissions.
//!
//! `Principal` is the closed-set type that connectors use to describe *who*
//! a permission applies to. It separates the *kind* of identity (user,
//! group, domain, anyone-public, anyone-with-link) from the *opaque
//! identifier* within each kind.
//!
//! See `docs/principal-mapping.md` for the semantic-mapping ADR explaining
//! when to reach for each variant and the design-pressure principle that
//! keeps the set closed.

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The closed set of identity kinds emitted on resource permissions.
///
/// Separates the *kind* of identity (user / group / domain / anyone-public /
/// anyone-with-link) from the *opaque identifier* within each kind. The kind
/// is what the policy engine reasons about; the identifier is what the
/// source system understands.
///
/// # Variants
///
/// - [`User`](Self::User) — individual identified by their primary email
///   from the source system.
/// - [`Group`](Self::Group) — named group whose membership is resolved by
///   the source system at policy-evaluation time. The inner string
///   typically carries a connector-prefix convention, e.g.,
///   `slack:CXXX`, for cross-connector identifier disambiguation in
///   shared policy rules. The SDK doesn't enforce this, but every
///   existing connector follows the convention.
/// - [`Domain`](Self::Domain) — anyone with an email under this domain at
///   the source.
/// - [`Anyone`](Self::Anyone) — publicly readable AND discoverable.
/// - [`AnyoneWithLink`](Self::AnyoneWithLink) — readable to whoever has
///   the resource URL; NOT discoverable. **Distinct from `Anyone` on
///   purpose** — collapsing them is a well-known search-platform bug
///   pattern (link-only docs leak into public search results).
///
/// `Hash + Eq` because the policy engine indexes resources by
/// permission-set membership; `HashSet<PermissionPredicate>` and
/// `HashMap<Principal, ...>` need to work directly.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Principal {
    /// Individual user identified by source-system identifier (email).
    User(String),
    /// Named group whose membership is resolved at policy time.
    Group(String),
    /// Domain whose members are anyone with an email under it.
    Domain(String),
    /// Publicly readable AND discoverable.
    Anyone,
    /// Readable to anyone with the URL; NOT discoverable.
    AnyoneWithLink,
}

impl Serialize for Principal {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::User(id) => format!("user:{id}"),
            Self::Group(id) => format!("group:{id}"),
            Self::Domain(d) => format!("domain:{d}"),
            Self::Anyone => "anyone".to_owned(),
            Self::AnyoneWithLink => "anyone-with-link".to_owned(),
        };
        s.serialize_str(&wire)
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // Legacy alias: "*" deserializes to Anyone. Documented in §9 of the
        // design spec as a one-way semantic rename — re-serialization
        // produces the canonical "anyone" form.
        if s == "*" || s == "anyone" {
            return Ok(Self::Anyone);
        }
        if s == "anyone-with-link" {
            return Ok(Self::AnyoneWithLink);
        }
        let (prefix, rest) = s.split_once(':').ok_or_else(|| {
            de::Error::custom(format!(
                "invalid Principal wire format: {s:?} \
                 (expected `kind:id` or `anyone`/`anyone-with-link`)"
            ))
        })?;
        match prefix {
            "user" => Ok(Self::User(rest.to_owned())),
            "group" => Ok(Self::Group(rest.to_owned())),
            "domain" => Ok(Self::Domain(rest.to_owned())),
            other => Err(de::Error::custom(format!(
                "unknown Principal kind: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_round_trip() {
        let p = Principal::User("alice@acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"user:alice@acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn group_round_trip() {
        let p = Principal::Group("eng@acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"group:eng@acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn domain_round_trip() {
        let p = Principal::Domain("acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"domain:acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn anyone_round_trip() {
        let p = Principal::Anyone;
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"anyone\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn anyone_with_link_round_trip() {
        let p = Principal::AnyoneWithLink;
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"anyone-with-link\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn legacy_star_deserializes_as_anyone() {
        let p: Principal = serde_json::from_str("\"*\"").unwrap();
        assert_eq!(p, Principal::Anyone);
    }

    #[test]
    fn legacy_star_round_trip_normalizes_to_anyone() {
        let p: Principal = serde_json::from_str("\"*\"").unwrap();
        let re = serde_json::to_string(&p).unwrap();
        assert_eq!(re, "\"anyone\"");
    }

    #[test]
    fn unknown_kind_error_includes_prefix() {
        let err = serde_json::from_str::<Principal>("\"weird:thing\"").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("weird"), "error should name the prefix: {msg}");
        assert!(msg.contains("unknown Principal kind"), "{msg}");
    }

    #[test]
    fn missing_colon_errors_on_deserialize() {
        let err = serde_json::from_str::<Principal>("\"justastring\"").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid Principal wire format"), "{msg}");
    }
}
