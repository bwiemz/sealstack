//! Output formatting for the CLI.
//!
//! Two modes:
//!
//! * [`Format::Human`] — padded ASCII tables + occasional free-form lines.
//!   This is the default. Stdout is assumed to be a TTY or log file.
//! * [`Format::Json`]  — raw JSON. The CLI writes the unwrapped gateway `data`
//!   value verbatim, preserving every field. Intended for scripting.
//!
//! The table implementation is intentionally minimalist — no `comfy-table`
//! dependency. For a small CLI, an 80-line hand-rolled formatter is easier to
//! maintain than threading theme / color / unicode options through every call
//! site.

use serde_json::Value;

/// Output mode chosen by the user (`--json` flag).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Format {
    /// Human-readable tables.
    Human,
    /// JSON verbatim.
    Json,
}

/// Print a JSON `Value` to stdout in the current format.
pub(crate) fn print(format: Format, value: &Value) {
    match format {
        Format::Json => {
            // Pretty-print for readability. Scripts that care about structure
            // can pipe through `jq -c` to compact.
            if let Ok(s) = serde_json::to_string_pretty(value) {
                println!("{s}");
            }
        }
        Format::Human => print_human(value),
    }
}

/// Print a table from an array-of-objects value, or print scalars verbatim.
fn print_human(value: &Value) {
    match value {
        Value::Array(items) if items.iter().all(|v| v.is_object()) => {
            print_table(items);
        }
        Value::Object(_) => {
            print_object_lines(value);
        }
        Value::String(s) => println!("{s}"),
        other => println!("{other}"),
    }
}

/// Render an array of JSON objects as a padded ASCII table.
fn print_table(items: &[Value]) {
    if items.is_empty() {
        println!("(no rows)");
        return;
    }
    // Column set = union of keys across all rows, in first-seen order.
    let mut columns: Vec<String> = Vec::new();
    for item in items {
        if let Some(obj) = item.as_object() {
            for k in obj.keys() {
                if !columns.iter().any(|c| c == k) {
                    columns.push(k.clone());
                }
            }
        }
    }

    // Cell values as strings.
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|c| {
                    item.as_object()
                        .and_then(|o| o.get(c))
                        .map(render_cell)
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();

    // Column widths.
    let mut widths: Vec<usize> = columns.iter().map(String::len).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    // Header.
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            print!("  ");
        }
        print!("{:width$}", col, width = widths[i]);
    }
    println!();

    // Separator.
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            print!("  ");
        }
        print!("{}", "-".repeat(*w));
    }
    println!();

    // Rows.
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                print!("  ");
            }
            print!("{:width$}", cell, width = widths[i]);
        }
        println!();
    }
}

/// Print an object as aligned `key: value` lines.
fn print_object_lines(value: &Value) {
    let Some(obj) = value.as_object() else {
        println!("{value}");
        return;
    };
    let key_width = obj.keys().map(String::len).max().unwrap_or(0);
    for (k, v) in obj {
        println!("{k:>key_width$}: {}", render_cell(v), key_width = key_width);
    }
}

/// Render a `Value` for a table cell or `key: value` line.
fn render_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => truncate(s, 80),
        Value::Array(a) => format!("[{} items]", a.len()),
        Value::Object(o) => {
            // Inline small objects; summarize large ones.
            if o.len() <= 2 {
                let pairs: Vec<String> = o
                    .iter()
                    .map(|(k, v)| format!("{k}={}", render_cell(v)))
                    .collect();
                pairs.join(", ")
            } else {
                format!("{{{} fields}}", o.len())
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_short_string_passes_through() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_ends_with_ellipsis() {
        let s = truncate("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn render_cell_summaries_are_short() {
        assert_eq!(render_cell(&json!(null)), "");
        assert_eq!(render_cell(&json!(true)), "true");
        assert_eq!(render_cell(&json!(42)), "42");
        assert_eq!(render_cell(&json!([1, 2, 3])), "[3 items]");
        let obj = render_cell(&json!({ "a": 1, "b": 2, "c": 3 }));
        assert_eq!(obj, "{3 fields}");
    }
}
