//! Small utilities shared across engine modules.

/// Returns `true` if `s` is a safe Postgres-compatible identifier.
///
/// Matches `[a-zA-Z_][a-zA-Z0-9_]*`. Used to gate table/column names that the
/// engine splices into generated SQL. The CSL compiler already constrains
/// identifier shape at parse time; this is defense-in-depth.
#[must_use]
pub(crate) fn is_safe_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_idents() {
        assert!(is_safe_ident("customer"));
        assert!(is_safe_ident("my_table_2"));
        assert!(is_safe_ident("_schema"));
    }

    #[test]
    fn invalid_idents() {
        assert!(!is_safe_ident(""));
        assert!(!is_safe_ident("customer; DROP TABLE"));
        assert!(!is_safe_ident("1customer"));
        assert!(!is_safe_ident("with space"));
        assert!(!is_safe_ident("quote'name"));
    }
}
