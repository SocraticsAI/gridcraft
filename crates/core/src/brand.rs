//! The product name GridCraft shows in its interface and writes into the files it saves (document
//! properties, PDF metadata, generated reports, the default comment author). A host that embeds
//! GridCraft under its own name sets it once, at startup, before any workbook opens.

use std::sync::OnceLock;

/// The name used until a host sets another.
pub const DEFAULT_NAME: &str = "GridCraft";

static NAME: OnceLock<String> = OnceLock::new();

/// The product name: the one the host set, else [`DEFAULT_NAME`].
pub fn name() -> &'static str {
    NAME.get().map(String::as_str).unwrap_or(DEFAULT_NAME)
}

/// Sets the product name for the rest of the process. Only the first call takes effect, so a name
/// can't change under open workbooks; returns false when a name was already set (or `name` is blank).
pub fn set_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && NAME.set(name.to_owned()).is_ok()
}
