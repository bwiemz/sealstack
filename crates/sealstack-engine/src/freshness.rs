//! Freshness decay score functions.
//!
//! Given an age (how old a record is, in seconds) and a strategy, produces a
//! multiplicative factor in `[0.0, 1.0]` applied to retrieval scores. The
//! schema registry carries the strategy per schema from the CSL
//! `context { freshness_decay = ... }` declaration.

use crate::schema_registry::FreshnessDecay;

/// Evaluate a freshness-decay factor.
///
/// * `strategy` — from the schema's context block.
/// * `age_secs` — age of the record in seconds. Negative values are clamped to zero.
#[must_use]
pub fn decay_factor(strategy: &FreshnessDecay, age_secs: i64) -> f32 {
    let age = age_secs.max(0) as f32;
    match strategy {
        FreshnessDecay::None => 1.0,
        FreshnessDecay::Exponential { half_life_secs } => {
            let half = *half_life_secs as f32;
            if half <= 0.0 {
                return 1.0;
            }
            let ratio = age / half;
            // 0.5 ^ ratio
            0.5f32.powf(ratio)
        }
        FreshnessDecay::Linear { window_secs } => {
            let window = *window_secs as f32;
            if window <= 0.0 {
                return 1.0;
            }
            (1.0 - (age / window)).max(0.0)
        }
        FreshnessDecay::Step { cliffs, factors } => {
            debug_assert_eq!(cliffs.len(), factors.len(), "cliffs/factors length mismatch");
            // Descending scan — apply the strictest (first crossed) factor.
            let mut factor = 1.0;
            for (cliff, fac) in cliffs.iter().zip(factors.iter()) {
                if age >= *cliff as f32 {
                    factor = *fac;
                }
            }
            factor
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_always_one() {
        assert_eq!(decay_factor(&FreshnessDecay::None, 0), 1.0);
        assert_eq!(decay_factor(&FreshnessDecay::None, 1_000_000), 1.0);
    }

    #[test]
    fn exponential_halves_at_half_life() {
        let s = FreshnessDecay::Exponential { half_life_secs: 100 };
        let f = decay_factor(&s, 100);
        assert!((f - 0.5).abs() < 1e-6, "got {f}");
    }

    #[test]
    fn linear_zero_at_window() {
        let s = FreshnessDecay::Linear { window_secs: 500 };
        assert!(decay_factor(&s, 500).abs() < 1e-6);
        assert!(decay_factor(&s, 1_000).abs() < 1e-6);
    }

    #[test]
    fn step_applies_threshold() {
        let s = FreshnessDecay::Step {
            cliffs: vec![10, 100, 1_000],
            factors: vec![0.9, 0.5, 0.1],
        };
        assert_eq!(decay_factor(&s, 5), 1.0);
        assert_eq!(decay_factor(&s, 50), 0.9);
        assert_eq!(decay_factor(&s, 500), 0.5);
        assert_eq!(decay_factor(&s, 5_000), 0.1);
    }
}
