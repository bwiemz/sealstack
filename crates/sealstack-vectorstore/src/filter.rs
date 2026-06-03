//! Generic filter DSL for vector-store queries.
//!
//! The engine threads CSL `@facet`-derived filters down into the vector
//! store on every search. Until v0.4, that filter was a flat
//! `{ "field": value }` AND-of-equals — enough for tenant scoping and
//! single-value facets, but the wire format couldn't express ranges,
//! sets, or boolean composition.
//!
//! [`Filter`] is the typed representation. JSON in and JSON out follow a
//! MongoDB-style operator vocabulary so existing consumers (the
//! retrieval module, the integration tests, the Python SDK) can move
//! incrementally — flat-equals still parses to `Filter::And(eq, eq,…)`.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "tenant": "acme",                     // shorthand: eq
//!   "status": { "$ne": "archived" },      // not equal
//!   "tag":    { "$in":  ["red", "blue"] }, // in set
//!   "priority": { "$gte": 3, "$lt": 8 }, // range; combined with AND
//!   "$or": [
//!     { "owner": "alice" },
//!     { "owner": "bob" }
//!   ],
//!   "$not": { "archived": true }
//! }
//! ```
//!
//! Multiple field entries at the top level AND together. Empty filter
//! ([`Filter::All`]) matches everything; the search path can short-circuit
//! around it.
//!
//! # Backend conversion
//!
//! - In-memory: [`Filter::matches`] over the chunk's metadata map.
//! - Qdrant: the qdrant module converts via pattern match; in v0.4 we
//!   handle Eq / In / Range / And explicitly; complex `$or`/`$not`
//!   compositions fall back to post-filter in memory (the qdrant module
//!   logs a warning so deployments notice when they're paying that cost).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

/// One filter node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Filter {
    /// `field == value`. `value` is a scalar (string, number, bool).
    Eq {
        /// Field path (top-level metadata key).
        field: String,
        /// Required value.
        value: Value,
    },
    /// `field != value`.
    Ne {
        /// Field path.
        field: String,
        /// Forbidden value.
        value: Value,
    },
    /// `field ∈ values`.
    In {
        /// Field path.
        field: String,
        /// Permitted set.
        values: Vec<Value>,
    },
    /// `field ∉ values`.
    NotIn {
        /// Field path.
        field: String,
        /// Forbidden set.
        values: Vec<Value>,
    },
    /// Numeric range.
    Range {
        /// Field path.
        field: String,
        /// Inclusive lower bound.
        gte: Option<f64>,
        /// Exclusive lower bound.
        gt: Option<f64>,
        /// Inclusive upper bound.
        lte: Option<f64>,
        /// Exclusive upper bound.
        lt: Option<f64>,
    },
    /// Logical AND.
    And(Vec<Filter>),
    /// Logical OR.
    Or(Vec<Filter>),
    /// Logical NOT.
    Not(Box<Filter>),
    /// Match every chunk.
    All,
}

/// Parse failures.
#[derive(Debug)]
pub enum FilterParseError {
    /// The top-level value wasn't an object.
    NotObject,
    /// An operator value had the wrong shape.
    BadOperator {
        /// The operator name (`$in`, `$gte`, …).
        op: String,
        /// Human-readable explanation.
        why: String,
    },
    /// An unknown operator.
    UnknownOperator(String),
}

impl fmt::Display for FilterParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "filter must be a JSON object"),
            Self::BadOperator { op, why } => write!(f, "operator `{op}`: {why}"),
            Self::UnknownOperator(op) => write!(f, "unknown operator `{op}`"),
        }
    }
}

impl std::error::Error for FilterParseError {}

impl Filter {
    /// Parse the MongoDB-style filter wire format into a typed [`Filter`].
    ///
    /// # Errors
    ///
    /// Returns [`FilterParseError`] when the input isn't a JSON object,
    /// or an operator's value doesn't match the expected shape.
    pub fn from_json(value: &Value) -> Result<Self, FilterParseError> {
        let Some(obj) = value.as_object() else {
            return Err(FilterParseError::NotObject);
        };
        if obj.is_empty() {
            return Ok(Self::All);
        }
        let mut parts: Vec<Filter> = Vec::new();
        for (key, value) in obj {
            parts.push(parse_kv(key, value)?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Self::And(parts)
        })
    }

    /// True if the supplied metadata map satisfies this filter.
    #[must_use]
    pub fn matches(&self, metadata: &Map<String, Value>) -> bool {
        match self {
            Self::All => true,
            Self::Eq { field, value } => metadata.get(field) == Some(value),
            Self::Ne { field, value } => metadata.get(field) != Some(value),
            Self::In { field, values } => {
                let Some(actual) = metadata.get(field) else {
                    return false;
                };
                values.iter().any(|v| v == actual)
            }
            Self::NotIn { field, values } => {
                let Some(actual) = metadata.get(field) else {
                    // Missing fields are vacuously not-in any set.
                    return true;
                };
                values.iter().all(|v| v != actual)
            }
            Self::Range {
                field,
                gte,
                gt,
                lte,
                lt,
            } => {
                let Some(actual) = metadata.get(field).and_then(Value::as_f64) else {
                    return false;
                };
                if let Some(g) = gte
                    && actual < *g
                {
                    return false;
                }
                if let Some(g) = gt
                    && actual <= *g
                {
                    return false;
                }
                if let Some(l) = lte
                    && actual > *l
                {
                    return false;
                }
                if let Some(l) = lt
                    && actual >= *l
                {
                    return false;
                }
                true
            }
            Self::And(parts) => parts.iter().all(|p| p.matches(metadata)),
            Self::Or(parts) => parts.iter().any(|p| p.matches(metadata)),
            Self::Not(inner) => !inner.matches(metadata),
        }
    }

    /// True if this filter only uses operators that the Qdrant backend
    /// can express natively: Eq / Ne / In / NotIn / Range / And. `$or`
    /// and `$not` compositions fall back to post-filtering. The qdrant
    /// module checks this so deployments paying the post-filter cost
    /// surface in the log.
    #[must_use]
    pub fn is_qdrant_native(&self) -> bool {
        match self {
            Self::All
            | Self::Eq { .. }
            | Self::Ne { .. }
            | Self::In { .. }
            | Self::NotIn { .. }
            | Self::Range { .. } => true,
            Self::And(parts) => parts.iter().all(Self::is_qdrant_native),
            Self::Or(_) | Self::Not(_) => false,
        }
    }
}

fn parse_kv(key: &str, value: &Value) -> Result<Filter, FilterParseError> {
    match key {
        "$and" => parse_compound(value, "and").map(Filter::And),
        "$or" => parse_compound(value, "or").map(Filter::Or),
        "$not" => Filter::from_json(value).map(|f| Filter::Not(Box::new(f))),
        field if field.starts_with('$') => Err(FilterParseError::UnknownOperator(field.to_owned())),
        field => parse_field(field, value),
    }
}

fn parse_compound(value: &Value, name: &str) -> Result<Vec<Filter>, FilterParseError> {
    let Some(arr) = value.as_array() else {
        return Err(FilterParseError::BadOperator {
            op: format!("${name}"),
            why: "expected an array of sub-filters".into(),
        });
    };
    arr.iter().map(Filter::from_json).collect()
}

fn parse_field(field: &str, value: &Value) -> Result<Filter, FilterParseError> {
    match value {
        Value::Object(ops) if has_any_op(ops) => parse_field_ops(field, ops),
        // Scalar or non-operator object → equality.
        _ => Ok(Filter::Eq {
            field: field.to_owned(),
            value: value.clone(),
        }),
    }
}

fn has_any_op(obj: &Map<String, Value>) -> bool {
    obj.keys().any(|k| k.starts_with('$'))
}

fn parse_field_ops(field: &str, ops: &Map<String, Value>) -> Result<Filter, FilterParseError> {
    let mut parts: Vec<Filter> = Vec::new();
    let mut range = RangeBuilder::default();
    for (op, value) in ops {
        match op.as_str() {
            "$eq" => parts.push(Filter::Eq {
                field: field.to_owned(),
                value: value.clone(),
            }),
            "$ne" => parts.push(Filter::Ne {
                field: field.to_owned(),
                value: value.clone(),
            }),
            "$in" => parts.push(Filter::In {
                field: field.to_owned(),
                values: as_array(op, value)?,
            }),
            "$nin" => parts.push(Filter::NotIn {
                field: field.to_owned(),
                values: as_array(op, value)?,
            }),
            "$gte" => range.gte = Some(as_number(op, value)?),
            "$gt" => range.gt = Some(as_number(op, value)?),
            "$lte" => range.lte = Some(as_number(op, value)?),
            "$lt" => range.lt = Some(as_number(op, value)?),
            other => return Err(FilterParseError::UnknownOperator(other.to_owned())),
        }
    }
    if range.any() {
        parts.push(Filter::Range {
            field: field.to_owned(),
            gte: range.gte,
            gt: range.gt,
            lte: range.lte,
            lt: range.lt,
        });
    }
    Ok(match parts.len() {
        0 => Filter::All,
        1 => parts.pop().unwrap(),
        _ => Filter::And(parts),
    })
}

#[derive(Default)]
struct RangeBuilder {
    gte: Option<f64>,
    gt: Option<f64>,
    lte: Option<f64>,
    lt: Option<f64>,
}

impl RangeBuilder {
    fn any(&self) -> bool {
        self.gte.is_some() || self.gt.is_some() || self.lte.is_some() || self.lt.is_some()
    }
}

fn as_array(op: &str, value: &Value) -> Result<Vec<Value>, FilterParseError> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| FilterParseError::BadOperator {
            op: op.to_owned(),
            why: "expected an array".into(),
        })
}

fn as_number(op: &str, value: &Value) -> Result<f64, FilterParseError> {
    value.as_f64().ok_or_else(|| FilterParseError::BadOperator {
        op: op.to_owned(),
        why: "expected a number".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn empty_filter_is_all() {
        let f = Filter::from_json(&json!({})).unwrap();
        assert!(matches!(f, Filter::All));
        assert!(f.matches(&map(&[])));
    }

    #[test]
    fn flat_eq_filter_is_backwards_compatible() {
        let f = Filter::from_json(&json!({ "tenant": "acme" })).unwrap();
        assert!(f.matches(&map(&[("tenant", json!("acme"))])));
        assert!(!f.matches(&map(&[("tenant", json!("other"))])));
        assert!(!f.matches(&map(&[])));
    }

    #[test]
    fn multiple_flat_entries_and_together() {
        let f = Filter::from_json(&json!({
            "tenant": "acme",
            "status": "open",
        }))
        .unwrap();
        assert!(matches!(f, Filter::And(_)));
        assert!(f.matches(&map(&[
            ("tenant", json!("acme")),
            ("status", json!("open")),
        ])));
        assert!(!f.matches(&map(&[
            ("tenant", json!("acme")),
            ("status", json!("closed")),
        ])));
    }

    #[test]
    fn ne_operator_excludes_match() {
        let f = Filter::from_json(&json!({ "status": { "$ne": "archived" } })).unwrap();
        assert!(f.matches(&map(&[("status", json!("open"))])));
        assert!(!f.matches(&map(&[("status", json!("archived"))])));
    }

    #[test]
    fn in_operator_passes_any_of_the_values() {
        let f = Filter::from_json(&json!({ "tag": { "$in": ["red", "blue"] } })).unwrap();
        assert!(f.matches(&map(&[("tag", json!("red"))])));
        assert!(f.matches(&map(&[("tag", json!("blue"))])));
        assert!(!f.matches(&map(&[("tag", json!("green"))])));
    }

    #[test]
    fn nin_operator_excludes_listed_values() {
        let f = Filter::from_json(&json!({ "tag": { "$nin": ["red"] } })).unwrap();
        assert!(f.matches(&map(&[("tag", json!("blue"))])));
        assert!(!f.matches(&map(&[("tag", json!("red"))])));
        // Missing field counts as not-in (vacuously true).
        assert!(f.matches(&map(&[])));
    }

    #[test]
    fn range_operators_combine_to_one_node() {
        let f = Filter::from_json(&json!({ "priority": { "$gte": 3, "$lt": 8 } })).unwrap();
        assert!(matches!(f, Filter::Range { .. }));
        assert!(f.matches(&map(&[("priority", json!(3))])));
        assert!(f.matches(&map(&[("priority", json!(7))])));
        assert!(!f.matches(&map(&[("priority", json!(2))])));
        assert!(!f.matches(&map(&[("priority", json!(8))])));
        // Missing numeric field never matches a range.
        assert!(!f.matches(&map(&[("priority", json!("nope"))])));
    }

    #[test]
    fn or_operator_short_circuits() {
        let f = Filter::from_json(&json!({
            "$or": [
                { "owner": "alice" },
                { "owner": "bob" }
            ]
        }))
        .unwrap();
        assert!(matches!(f, Filter::Or(_)));
        assert!(f.matches(&map(&[("owner", json!("alice"))])));
        assert!(f.matches(&map(&[("owner", json!("bob"))])));
        assert!(!f.matches(&map(&[("owner", json!("charlie"))])));
    }

    #[test]
    fn not_operator_inverts_a_filter() {
        let f = Filter::from_json(&json!({ "$not": { "archived": true } })).unwrap();
        assert!(matches!(f, Filter::Not(_)));
        assert!(f.matches(&map(&[])));
        assert!(f.matches(&map(&[("archived", json!(false))])));
        assert!(!f.matches(&map(&[("archived", json!(true))])));
    }

    #[test]
    fn unknown_field_operator_rejected() {
        let err = Filter::from_json(&json!({ "x": { "$contains": "y" } })).unwrap_err();
        assert!(matches!(err, FilterParseError::UnknownOperator(_)));
    }

    #[test]
    fn unknown_top_level_operator_rejected() {
        let err = Filter::from_json(&json!({ "$weird": 1 })).unwrap_err();
        assert!(matches!(err, FilterParseError::UnknownOperator(_)));
    }

    #[test]
    fn non_array_in_value_rejected() {
        let err = Filter::from_json(&json!({ "tag": { "$in": "not_array" } })).unwrap_err();
        assert!(matches!(err, FilterParseError::BadOperator { .. }));
    }

    #[test]
    fn non_object_input_rejected() {
        let err = Filter::from_json(&json!([1, 2, 3])).unwrap_err();
        assert!(matches!(err, FilterParseError::NotObject));
    }

    #[test]
    fn is_qdrant_native_distinguishes_supported_ops() {
        let eq = Filter::from_json(&json!({ "tenant": "acme" })).unwrap();
        assert!(eq.is_qdrant_native());
        let combined = Filter::from_json(&json!({
            "tenant": "acme",
            "priority": { "$gte": 3 },
            "tag": { "$in": ["red"] }
        }))
        .unwrap();
        assert!(combined.is_qdrant_native());
        let or = Filter::from_json(&json!({ "$or": [{"a": 1}, {"b": 2}] })).unwrap();
        assert!(!or.is_qdrant_native());
        let not_eq = Filter::from_json(&json!({ "$not": { "a": 1 } })).unwrap();
        assert!(!not_eq.is_qdrant_native());
    }
}
