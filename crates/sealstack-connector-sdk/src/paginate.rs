//! Pagination primitives for connectors.
//!
//! **Users should reach for one of the three reference builders first:**
//! - [`BodyCursorPaginator`] — cursor is in the response body (Slack, Drive,
//!   Notion, Linear).
//! - [`LinkHeaderPaginator`] — cursor is in a `Link: rel="next"` header
//!   (GitHub, GitLab).
//! - [`OffsetPaginator`] — numeric `start`/`limit` with a total (Jira,
//!   Confluence).
//!
//! The [`Paginator`] trait is the extension point for APIs that do not fit
//! any of these three patterns (e.g. Stripe's `starts_with`).
//!
//! Reference builders land in subsequent tasks; this module currently
//! exports only the trait and the [`paginate`] adapter.

use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;

use sealstack_common::{SealStackError, SealStackResult};

use crate::http::HttpClient;

/// A paginator drives the SDK's stream adapter, returning one page of
/// deserialized items at a time plus the cursor for the next page (or
/// `None` to terminate).
///
/// # Contract
///
/// - Empty pages with a valid next cursor are expected; implementations must
///   not short-circuit on `items.is_empty()`. Continue until `next_cursor ==
///   None`.
/// - Once [`Paginator::fetch_page`] returns `Err`, the paginator is
///   considered poisoned. The stream adapter will not call `fetch_page`
///   again. Implementations do not need to handle re-entry after failure.
/// - Returning the same cursor twice consecutively is a paginator bug.
///   The adapter detects this and returns
///   [`SealStackError::PaginatorCursorLoop`].
///
/// # Ownership
///
/// Paginators are passed by value to the adapter (`paginate(paginator,
/// client)`). They may hold non-cloneable or expensive-to-clone state (TCP
/// sessions, dedupe sets) — do NOT add a `Clone` bound or copy the
/// paginator anywhere in the adapter.
#[async_trait]
pub trait Paginator: Send + 'static {
    /// Item yielded by the stream — typically a deserialized DTO.
    type Item: Send + 'static;

    /// Fetch one page, given the cursor from the previous page (None on
    /// first call). Returns the page's items and the next cursor (None when
    /// the stream is exhausted).
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<Self::Item>, Option<String>)>;
}

/// Drive a [`Paginator`] to exhaustion, yielding items as they're fetched.
///
/// The adapter:
/// - Calls `fetch_page` with `None` initially, then with each returned cursor.
/// - Yields each item one at a time across pages.
/// - Detects cursor non-advancement (same cursor returned twice consecutively)
///   and yields `SealStackError::PaginatorCursorLoop` instead of looping
///   forever.
/// - Stops calling `fetch_page` after the first `Err` (paginator poisoned).
pub fn paginate<P: Paginator>(
    mut paginator: P,
    client: Arc<HttpClient>,
) -> std::pin::Pin<Box<dyn Stream<Item = SealStackResult<P::Item>> + Send>> {
    Box::pin(async_stream::try_stream! {
        let mut cursor: Option<String> = None;
        let mut prev_cursor: Option<String> = None;
        loop {
            // Detect cursor non-advancement before fetching: if the cursor we
            // are about to use is the same as the one we used on the previous
            // iteration, the paginator is stuck.
            if let (Some(p), Some(c)) = (&prev_cursor, &cursor)
                && p == c
            {
                Err(SealStackError::PaginatorCursorLoop { cursor: c.clone() })?;
            }
            let (items, next) = paginator.fetch_page(&client, cursor.clone()).await?;
            for it in items {
                yield it;
            }
            prev_cursor = cursor.clone();
            match next {
                None => break,
                Some(n) => {
                    cursor = Some(n);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StaticToken;
    use crate::retry::RetryPolicy;
    use futures::StreamExt;

    /// Yields two pages then exhausts.
    struct TwoPage {
        calls: u32,
    }

    #[async_trait]
    impl Paginator for TwoPage {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            self.calls += 1;
            match cursor.as_deref() {
                None => Ok((vec![1, 2], Some("p2".into()))),
                Some("p2") => Ok((vec![3, 4], None)),
                Some(other) => panic!("unexpected cursor {other}"),
            }
        }
    }

    /// Yields an empty page with a next cursor, then the real page.
    struct EmptyThenFull;

    #[async_trait]
    impl Paginator for EmptyThenFull {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            match cursor.as_deref() {
                None => Ok((vec![], Some("next".into()))),
                Some("next") => Ok((vec![42], None)),
                Some(other) => panic!("unexpected {other}"),
            }
        }
    }

    /// Returns the same cursor twice — triggers `PaginatorCursorLoop`.
    struct StuckCursor;

    #[async_trait]
    impl Paginator for StuckCursor {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            _cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            Ok((vec![1], Some("same".into())))
        }
    }

    fn dummy_client() -> Arc<HttpClient> {
        Arc::new(
            HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default())
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn drives_paginator_to_exhaustion() {
        let items: Vec<_> = paginate(TwoPage { calls: 0 }, dummy_client())
            .collect()
            .await;
        let ok: Vec<u32> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(ok, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn continues_through_empty_page() {
        let items: Vec<_> = paginate(EmptyThenFull, dummy_client()).collect().await;
        let ok: Vec<u32> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(ok, vec![42]);
    }

    #[tokio::test]
    async fn detects_cursor_loop() {
        let mut s = paginate(StuckCursor, dummy_client());
        // First page yields 1 (with cursor "same").
        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        // Second page yields 1 again (cursor "same" repeated → loop).
        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        // Third fetch yields the loop error.
        let err = s.next().await.unwrap().unwrap_err();
        assert!(
            matches!(err, SealStackError::PaginatorCursorLoop { .. }),
            "expected PaginatorCursorLoop, got {err}"
        );
    }
}
