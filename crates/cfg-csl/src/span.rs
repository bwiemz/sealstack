//! Source position tracking.
//!
//! We track spans as half-open byte ranges against the original source. The
//! `Located` wrapper in `winnow` gives us these essentially for free during
//! parsing; later passes carry them forward for diagnostics.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// A half-open byte range `[start, end)` into the source text.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Span {
    /// Inclusive start, in bytes.
    pub start: usize,
    /// Exclusive end, in bytes.
    pub end: usize,
}

impl Span {
    /// Construct a span from start and end byte offsets.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// A zero-width span at the given position. Useful for "point" diagnostics.
    #[must_use]
    pub const fn point(at: usize) -> Self {
        Self::new(at, at)
    }

    /// Merge two spans into the minimum span that covers both.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// Length of the span in bytes.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    /// Whether the span is zero-width.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl From<Range<usize>> for Span {
    fn from(r: Range<usize>) -> Self {
        Self::new(r.start, r.end)
    }
}

impl From<Span> for Range<usize> {
    fn from(s: Span) -> Self {
        s.start..s.end
    }
}

impl From<Span> for miette::SourceSpan {
    fn from(s: Span) -> Self {
        miette::SourceSpan::from(s.start..s.end)
    }
}
