// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links: where a hard-coded value came from ([`SourceLink`]), kept per sheet in
//! [`Sheet::sources`]. The map is shared, not copied, between a workbook and its undo snapshots
//! until a command changes a link ([`Sheet::sources_mut`]), and links to one document share its
//! name, so an edit that leaves the links alone costs nothing however many there are.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Sheet;

/// A sheet's source links, by cell.
pub type SourceLinks = BTreeMap<CellRef, SourceLink>;

/// Where a hard-coded value came from: a document and the place in it. A formula has none of its
/// own. Moves with its cell like a note; typing over the cell drops it. Unknown fields are ignored
/// on load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLink {
    /// The source document, as a relative path inside the open folder (see
    /// [`SourceLink::relative_path`]). Links to one document share the name.
    ///
    /// Untrusted when it comes from a file: a workbook from anywhere can name an absolute path, a
    /// UNC share or a `..` escape. `source.set` refuses such a path and the `.xlsx` reader drops
    /// a link naming one, but a host that opens or fetches what a link names must still resolve
    /// it against its own root and refuse anything that leaves it (a symbolic link can).
    pub file: Arc<str>,
    /// Where in the document: `Sheet!A1`, `page:N`, `slide:N`, `line:N` or `document` (the whole file).
    pub locator: String,
}

impl SourceLink {
    /// The link's `file` when it is a plain relative path: see [`SourceLink::is_relative_path`].
    pub fn relative_path(&self) -> Option<&str> {
        Self::is_relative_path(&self.file).then_some(&*self.file)
    }

    /// Whether `path` is a plain relative path that stays inside the folder it is resolved
    /// against: `/`-separated segments, none of them empty, `.` or `..`; no backslash (a Windows
    /// separator, and the start of a UNC share); no `:` in the first segment (a drive letter such
    /// as `C:` or a URL scheme such as `file:` or `https:`); no control character (U+0000–U+001F,
    /// U+007F–U+009F). The path is taken as it is: a host must not percent-decode it.
    pub fn is_relative_path(path: &str) -> bool {
        if path.is_empty() || path.contains('\\') || path.chars().any(char::is_control) {
            return false;
        }
        let mut segments = path.split('/');
        if segments.clone().next().is_some_and(|first| first.contains(':')) {
            return false;
        }
        segments.all(|s| !s.is_empty() && s != "." && s != "..")
    }
}

impl SourceLink {
    /// Makes the links that name the same document share one copy of its name.
    pub fn share_files<'a>(links: impl IntoIterator<Item = &'a mut SourceLink>) {
        let mut names: BTreeSet<Arc<str>> = BTreeSet::new();
        for link in links {
            match names.get(&link.file) {
                Some(name) => link.file = name.clone(),
                None => {
                    names.insert(link.file.clone());
                }
            }
        }
    }
}

impl Sheet {
    /// The source links, to change them. The map is shared with undo snapshots until this copies
    /// it, so call it only once a link is known to change.
    pub fn sources_mut(&mut self) -> &mut SourceLinks {
        Arc::make_mut(&mut self.sources)
    }

    /// Removes the links in `range` that `drop` names, and returns how many went. The shared map
    /// is copied only when one goes, and only the links in `range`'s rows are looked at.
    pub fn remove_sources_in(&mut self, range: RangeRef, drop: impl Fn(CellRef) -> bool) -> usize {
        let range = RangeRef::new(range.start, range.end);
        let cols = range.start.col..=range.end.col;
        let gone: Vec<CellRef> = self.sources.range(range.start..=range.end).map(|(c, _)| *c).filter(|c| cols.contains(&c.col) && drop(*c)).collect();
        if !gone.is_empty() {
            let links = self.sources_mut();
            for c in &gone {
                links.remove(c);
            }
        }
        gone.len()
    }
}

/// Whether a sheet has no source links (its JSON then has no `sources` key).
pub(crate) fn no_links(links: &Arc<SourceLinks>) -> bool {
    links.is_empty()
}

/// Serde for [`Sheet::sources`]: a map keyed by `A1` text, as `crate::cellmap` writes one.
pub(crate) mod links_serde {
    use super::*;

    pub fn serialize<S: Serializer>(links: &Arc<SourceLinks>, s: S) -> Result<S::Ok, S::Error> {
        crate::cellmap::serialize(links, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<SourceLinks>, D::Error> {
        let mut links: SourceLinks = crate::cellmap::deserialize(d)?;
        // Links to one document share its name, as they do once read from an `.xlsx`.
        SourceLink::share_files(links.values_mut());
        Ok(Arc::new(links))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(file: &str) -> SourceLink {
        SourceLink { file: file.into(), locator: "page:1".into() }
    }

    #[test]
    fn plain_relative_paths_are_accepted() {
        for p in [
            "Statement.pdf",
            "Cash/Bank Statement September 2019.xlsx",
            "a/b/c.d.pdf",
            "Q3/notes: draft.pdf",
            "50% split.pdf",
            "Café/Résumé.docx",
            "...pdf",
        ] {
            assert!(SourceLink::is_relative_path(p), "{p}");
            assert_eq!(link(p).relative_path(), Some(p));
        }
    }

    #[test]
    fn paths_that_leave_the_folder_or_name_another_place_are_refused() {
        for p in [
            "",
            "/etc/passwd",
            "/",
            "C:/Windows/win.ini",
            "C:Windows",
            "c:",
            "\\\\attacker\\share\\x.pdf",
            "//attacker/share/x.pdf",
            "Cash\\x.pdf",
            "file:///etc/passwd",
            "file:x.pdf",
            "http://example.com/x.pdf",
            "https:x",
            "mailto:a@b.c",
            "../x.pdf",
            "a/../../x.pdf",
            "a/..",
            "./x.pdf",
            "a/./b.pdf",
            "a//b.pdf",
            "a/",
            "a\u{0}b.pdf",
            "a\nb.pdf",
            "a\u{1F}b.pdf",
            "a\u{7F}b.pdf",
            "a\u{85}b.pdf",
        ] {
            assert!(!SourceLink::is_relative_path(p), "{p:?}");
            assert_eq!(link(p).relative_path(), None, "{p:?}");
        }
    }

    #[test]
    fn removing_links_copies_the_shared_map_only_when_one_goes() {
        let mut sh = Sheet::new("S");
        sh.sources_mut().insert(CellRef::new(1, 1), link("a.pdf"));
        sh.sources_mut().insert(CellRef::new(5, 5), link("a.pdf"));
        let snapshot = sh.clone();
        // Nothing in the range: still shared.
        assert_eq!(sh.remove_sources_in(RangeRef::new(CellRef::new(2, 0), CellRef::new(4, 9)), |_| true), 0);
        assert_eq!(sh.remove_sources_in(RangeRef::new(CellRef::new(0, 2), CellRef::new(9, 4)), |_| true), 0);
        assert!(Arc::ptr_eq(&sh.sources, &snapshot.sources));
        // A backwards range is read the right way round.
        assert_eq!(sh.remove_sources_in(RangeRef { start: CellRef::new(5, 5), end: CellRef::new(1, 1) }, |c| c.row == 5), 1);
        assert!(!Arc::ptr_eq(&sh.sources, &snapshot.sources));
        assert_eq!(sh.sources.len(), 1);
        assert_eq!(snapshot.sources.len(), 2, "the snapshot keeps its links");
    }

    #[test]
    fn source_links_survive_json_and_stay_out_of_it_when_empty() {
        let mut s = Sheet::new("S");
        let empty = serde_json::to_string(&s).expect("serialize");
        assert!(!empty.contains("\"sources\""), "an unlinked sheet writes no sources key: {empty}");
        s.sources_mut().insert(CellRef::new(1, 1), link("Cash/Statement.pdf"));
        s.sources_mut().insert(CellRef::new(2, 1), link("Cash/Statement.pdf"));
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("\"B2\""), "keys are A1 text: {json}");
        let back: Sheet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.sources, s.sources);
        let files: Vec<&Arc<str>> = back.sources.values().map(|l| &l.file).collect();
        assert!(Arc::ptr_eq(files[0], files[1]), "links to one document share its name");
        // A link with a field this version doesn't know still loads, without it.
        let mut value = serde_json::to_value(&s).expect("to value");
        value["sources"]["B2"]["unknown"] = serde_json::json!("x");
        let unknown: Sheet = serde_json::from_value(value.clone()).expect("a link with an unknown field loads");
        assert_eq!(unknown.sources, s.sources);
        // JSON written before links existed still loads.
        value.as_object_mut().expect("object").remove("sources");
        let old: Sheet = serde_json::from_value(value).expect("older JSON loads");
        assert!(old.sources.is_empty());
    }
}
