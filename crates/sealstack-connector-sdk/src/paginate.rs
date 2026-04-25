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
use serde::de::DeserializeOwned;

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

/// Body-cursor paginator: the cursor lives inside the response body.
///
/// Use for Slack (`response_metadata.next_cursor`), Google Drive
/// (`nextPageToken`), Notion (`next_cursor`), Linear (`pageInfo.endCursor`).
///
/// # Type parameters
///
/// - `T` — deserialized item type.
/// - `Req` — closure that builds a [`reqwest::RequestBuilder`] given the
///   client and the current cursor (`None` on the first call).
/// - `EI` — closure that extracts the page's item array from the response JSON.
/// - `EC` — closure that extracts the next-page cursor from the response JSON.
///   Return `None` to terminate the stream.
// clippy::type_complexity: the four-generic struct is intentional — each
// generic represents a distinct user-supplied closure, not incidental
// nesting.
#[allow(clippy::type_complexity)]
pub struct BodyCursorPaginator<T, Req, EI, EC>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
    EI: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    EC: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    request: Req,
    extract_items: EI,
    extract_cursor: EC,
    // PhantomData<fn() -> T>: marks T as used at the type level only, making
    // the struct covariant in T without owning a T value.
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req, EI, EC> BodyCursorPaginator<T, Req, EI, EC>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
    EI: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    EC: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    /// Build a new paginator.
    ///
    /// - `request`: closure that composes a request given the client and the
    ///   current cursor (or `None` on the first call).
    /// - `extract_items`: extracts the page's item array from the response
    ///   JSON.
    /// - `extract_cursor`: extracts the next-page cursor from the response
    ///   JSON. Return `None` to terminate the stream.
    pub fn new(request: Req, extract_items: EI, extract_cursor: EC) -> Self {
        Self {
            request,
            extract_items,
            extract_cursor,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req, EI, EC> Paginator for BodyCursorPaginator<T, Req, EI, EC>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
    EI: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    EC: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    type Item = T;

    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, cursor.as_deref());
        let resp = client.send(rb).await?;
        let body: serde_json::Value = resp.json().await?;
        let items = (self.extract_items)(&body)?;
        let next = (self.extract_cursor)(&body);
        Ok((items, next))
    }
}

/// Parse a `Link` header value and return the URL of the `rel="next"` entry.
///
/// Used by [`LinkHeaderPaginator`]; `pub` so connectors with Link-like
/// custom headers can reuse it.
#[must_use]
pub fn next_link(header: &str) -> Option<String> {
    for part in header.split(',') {
        let part = part.trim();
        let mut it = part.split(';');
        let url_bracket = it.next()?.trim();
        let url = url_bracket.strip_prefix('<')?.strip_suffix('>')?;
        let mut is_next = false;
        for attr in it {
            let attr = attr.trim();
            if attr == "rel=\"next\"" || attr == "rel=next" {
                is_next = true;
                break;
            }
        }
        if is_next {
            return Some(url.to_owned());
        }
    }
    None
}

/// Link-header paginator: cursor is the URL from `Link: rel="next"`.
///
/// Use for GitHub REST, GitLab REST, anything following RFC 8288 link headers.
///
/// # Type parameters
///
/// - `T` — deserialized item type. The response body is parsed as `Vec<T>`.
/// - `Req` — closure that builds a request given the client and the current
///   cursor URL (`None` on the first call).
pub struct LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
{
    request: Req,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req> LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
{
    /// Build a new paginator.
    ///
    /// `request` receives `cursor = None` on the first call; subsequent
    /// calls receive the URL extracted from the previous response's
    /// `Link: rel="next"` header.
    pub fn new(request: Req) -> Self {
        Self {
            request,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req> Paginator for LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder + Send + 'static,
{
    type Item = T;

    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, cursor.as_deref());
        let resp = client.send(rb).await?;
        // Extract the Link header before consuming the response body.
        let next = resp.header("Link").and_then(next_link);
        let items: Vec<T> = resp.json().await?;
        Ok((items, next))
    }
}

/// Offset/limit paginator: numeric `start` + `limit` against a `total`.
///
/// Use for Jira REST API v3 (`startAt`/`maxResults`) and Confluence Cloud
/// REST (`start`/`limit`). The response body must include both the item
/// array (under a configurable JSON key) and a numeric `total` at the top
/// level — the paginator uses `total` to detect exhaustion. If `total` is
/// absent, the paginator stops when a page returns fewer items than `limit`.
///
/// # Type parameters
///
/// - `T` — deserialized item type.
/// - `Req` — closure that builds a request given the client, the current
///   `start` offset, and the configured `limit`.
pub struct OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    limit: u64,
    request: Req,
    items_key: &'static str,
    next_start: u64,
    total: Option<u64>,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req> OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    /// Build a new paginator with the given page size, request closure, and
    /// response key under which items live.
    pub fn new(limit: u64, request: Req, items_key: &'static str) -> Self {
        Self {
            limit,
            request,
            items_key,
            next_start: 0,
            total: None,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req> Paginator for OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    type Item = T;

    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        _cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, self.next_start, self.limit);
        let resp = client.send(rb).await?;
        let body: serde_json::Value = resp.json().await?;

        if self.total.is_none() {
            self.total = body.get("total").and_then(serde_json::Value::as_u64);
        }
        let items: Vec<T> = body
            .get(self.items_key)
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                SealStackError::backend(format!("missing array at `{}`", self.items_key))
            })?
            .iter()
            .map(|x| {
                serde_json::from_value::<T>(x.clone())
                    .map_err(|e| SealStackError::backend(format!("{e}")))
            })
            .collect::<SealStackResult<Vec<_>>>()?;

        let returned = items.len() as u64;
        self.next_start += returned;

        let done = match self.total {
            Some(total) => self.next_start >= total,
            // No total in response — stop when server returns fewer than limit.
            None => returned < self.limit,
        };

        // The adapter threads cursors by value equality; use the offset as a
        // stringified cursor so the cursor-loop detector works (each new offset
        // produces a different cursor string).
        let next = if done {
            None
        } else {
            Some(self.next_start.to_string())
        };
        Ok((items, next))
    }
}

#[cfg(test)]
mod link_header_tests {
    use super::next_link;

    #[test]
    fn parses_next_link() {
        let hdr = r#"<https://api.example.com/p?page=2>; rel="next", <https://api.example.com/p?page=9>; rel="last""#;
        assert_eq!(
            next_link(hdr),
            Some("https://api.example.com/p?page=2".to_owned())
        );
    }

    #[test]
    fn no_next_link_returns_none() {
        let hdr = r#"<https://api.example.com/p?page=9>; rel="last""#;
        assert_eq!(next_link(hdr), None);
    }

    #[test]
    fn parses_unquoted_rel_next() {
        let hdr = "<https://api.example.com/p?page=2>; rel=next";
        assert_eq!(
            next_link(hdr),
            Some("https://api.example.com/p?page=2".to_owned())
        );
    }

    #[test]
    fn rejects_malformed_bracket() {
        // Missing closing `>` — should not match.
        let hdr = r#"<https://api.example.com; rel="next""#;
        assert_eq!(next_link(hdr), None);
    }
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
        Arc::new(HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap())
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

    /// Single-page response — first call returns items + None.
    struct SinglePage;

    #[async_trait]
    impl Paginator for SinglePage {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            assert!(cursor.is_none(), "single-page test should only fetch once");
            Ok((vec![7, 8, 9], None))
        }
    }

    #[tokio::test]
    async fn single_page_response() {
        let items: Vec<_> = paginate(SinglePage, dummy_client()).collect().await;
        let ok: Vec<u32> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(ok, vec![7, 8, 9]);
    }

    /// First call returns empty items and None — the stream terminates with
    /// zero items.
    struct EmptyTerminal;

    #[async_trait]
    impl Paginator for EmptyTerminal {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            _cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            Ok((vec![], None))
        }
    }

    #[tokio::test]
    async fn empty_first_page_terminates() {
        let items: Vec<_> = paginate(EmptyTerminal, dummy_client()).collect().await;
        assert!(items.is_empty(), "expected empty stream, got {items:?}");
    }

    /// Yields one page of items, then errors. The adapter must yield the
    /// items, then yield the error, then never call `fetch_page` again.
    struct ErrorAfterFirst {
        calls: u32,
    }

    #[async_trait]
    impl Paginator for ErrorAfterFirst {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            self.calls += 1;
            assert!(self.calls <= 2, "adapter must not re-enter after error");
            match cursor.as_deref() {
                None => Ok((vec![1, 2], Some("p2".into()))),
                Some("p2") => Err(SealStackError::backend("upstream blew up")),
                Some(other) => panic!("unexpected cursor {other}"),
            }
        }
    }

    #[tokio::test]
    async fn error_mid_stream_poisons_paginator() {
        let mut s = paginate(ErrorAfterFirst { calls: 0 }, dummy_client());
        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        assert_eq!(s.next().await.unwrap().unwrap(), 2);
        let err = s.next().await.unwrap().unwrap_err();
        assert!(matches!(err, SealStackError::Backend(_)), "got {err}");
        // After Err, the stream is done — adapter must not call fetch_page again.
        assert!(s.next().await.is_none());
    }
}
