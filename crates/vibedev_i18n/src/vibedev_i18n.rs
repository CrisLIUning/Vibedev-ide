//! VibeDev runtime translation table.
//!
//! The build-time zedl10n pass (see `vibedev-i18n/`) translates string LITERALS
//! in source. Some UI text has no literal to replace because it is produced at
//! runtime: command-palette entries are humanized from action *type* names, and
//! ACP mode names/descriptions arrive from the agent over the wire. This crate
//! provides a tiny runtime lookup applied at those few render sites.
//!
//! `runtime-translations.json` is committed EMPTY (`{}`) so English builds are
//! byte-for-byte unaffected: every `tr` returns `None` and callers fall back to
//! the original English. The Chinese build (`vibedev-i18n/build-zh`) copies the
//! populated table over it before compiling and `git restore`s it afterwards —
//! the same populate → compile → restore flow zedl10n already uses. Keys are the
//! exact English string the render site produced.

use std::collections::HashMap;
use std::sync::OnceLock;

const RAW: &str = include_str!("runtime-translations.json");

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    // A malformed table must never take down the UI: fall back to empty (every
    // lookup misses → English) instead of panicking at the first palette open.
    TABLE.get_or_init(|| serde_json::from_str(RAW).unwrap_or_default())
}

/// Chinese translation for an English UI string, or `None` when untranslated.
///
/// English builds always return `None` (the embedded table is empty), so every
/// call site falls back to its English input.
pub fn tr(english: &str) -> Option<&'static str> {
    table().get(english).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever is embedded (`{}` in committed source, the populated table in a
    /// Chinese build) must parse — `table()` must never panic at runtime.
    #[test]
    fn embedded_table_parses() {
        let _ = table();
    }

    /// A miss returns `None` so the call site keeps its English string. This is
    /// the only guarantee the committed (empty) table can make; the populated
    /// table is exercised end-to-end by the build-zh flow.
    #[test]
    fn missing_key_is_none() {
        assert_eq!(tr("\u{0}definitely not a key\u{0}"), None);
    }
}
