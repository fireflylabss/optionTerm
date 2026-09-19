//! Case-insensitive substring search over the whole screen + scrollback.
//!
//! Ported from optionTerm 0.1.x. Rows are formatted one at a time so every
//! hit keeps its exact `(row, column)` coordinates, which is what the caller
//! needs to scroll to and select it.

use anyhow::anyhow;
use libghostty_vt::{
    Terminal,
    fmt::{Formatter, FormatterOptions},
    selection::Selection,
    terminal::{Point, PointCoordinate},
};

use crate::frame::Match;

/// Bail out rather than freeze the pane thread on a pathological scrollback.
const MAX_MATCHES: usize = 2_000;

/// All hits for `needle`, in screen-space rows (scrollback + viewport).
pub fn search_terminal(terminal: &Terminal<'_, '_>, cols: u16, needle: &str) -> Vec<Match> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_lower = needle.to_lowercase();
    let width = needle.chars().count() as u16;
    let Ok(total) = terminal.total_rows() else {
        return Vec::new();
    };
    let last_col = cols.saturating_sub(1);

    let mut matches = Vec::new();
    let mut line = String::new();
    for row in 0..total as u32 {
        line.clear();
        if !row_text(terminal, row, last_col, &mut line) {
            continue;
        }
        let haystack = line.to_lowercase();
        let mut from = 0usize;
        while let Some(rel) = haystack[from..].find(&needle_lower) {
            let byte = from + rel;
            // The grid is indexed by cell, so convert the byte offset to a
            // character count before reporting a column.
            let col = line[..byte].chars().count() as u16;
            matches.push(Match {
                row: row.min(u16::MAX as u32) as u16,
                col,
                width,
            });
            if matches.len() >= MAX_MATCHES {
                return matches;
            }
            from = byte + needle_lower.len().max(1);
            if from >= haystack.len() {
                break;
            }
        }
    }
    matches
}

/// Format a single screen row (scrollback included) into `out`.
fn row_text(terminal: &Terminal<'_, '_>, y: u32, last_col: u16, out: &mut String) -> bool {
    let (Ok(start), Ok(end)) = (
        terminal.grid_ref(Point::Screen(PointCoordinate { x: 0, y })),
        terminal.grid_ref(Point::Screen(PointCoordinate { x: last_col, y })),
    ) else {
        return false;
    };
    let sel = Selection::new(start, end, false);
    let opts = FormatterOptions::new().with_selection(&sel).with_trim(true);
    let Ok(mut formatter) = Formatter::new(terminal, opts) else {
        return false;
    };
    let Ok(bytes) = formatter.format_alloc(None) else {
        return false;
    };
    out.push_str(String::from_utf8_lossy(&bytes).trim_end_matches('\n'));
    !out.is_empty()
}

/// Text of a selection, formatted the same way copy uses it.
pub fn selection_text(terminal: &Terminal<'_, '_>, sel: &Selection<'_>) -> Option<String> {
    let opts = FormatterOptions::new().with_selection(sel).with_trim(true);
    let mut formatter = Formatter::new(terminal, opts)
        .map_err(|e| anyhow!("{e:?}"))
        .ok()?;
    let bytes = formatter.format_alloc(None).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (!text.is_empty()).then_some(text)
}
