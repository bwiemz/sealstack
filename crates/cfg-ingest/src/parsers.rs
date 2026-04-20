//! Parsers for the formats connectors emit (Markdown, PDF, DOCX, code, etc.).
//!
//! Pre-scaffold pass-throughs — the engine treats the `Resource.body` string
//! as-is today. A v0.2 pass plugs in real parsers via the `kind` field (e.g.
//! `"pdf"` → `pdf_to_text`; `"docx"` → `docx_to_text`).

/// Markdown pass-through. Kept as a function so future implementations
/// (stripping frontmatter, resolving link references, etc.) can drop in
/// without changing every call site.
#[must_use]
pub fn markdown_to_text(src: &str) -> String {
    src.to_owned()
}
