// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links: which document each hard-coded value came from (see
//! `gridcraft_model::SourceLink`). A host that fills cells from its documents writes the values,
//! then their links (`source.set`); typing over a cell drops its link. A formula cell takes none.

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::SourceLink;
use serde_json::{Value as Json, json};

use super::*;
use crate::Session;

/// Most cells one `source.set` may name.
const MAX_CELLS: u64 = 1_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "source.set",
            "Set Cell Sources",
            [],
            None,
            "{sheet?, range: \"A1:C3\", links: [[{file, locator} | null]] (one row per range row; null removes a link; file is a relative path; a formula cell takes none)} → {linked}",
            has_doc,
            set_links
        ),
        cmd!(query "source.get", "Cell Source", [], None, "{sheet?, cell: \"B2\"} → {link: {file, locator} | null}", has_doc, get_link),
        cmd!("source.clear", "Clear Cell Sources", [], None, "{sheet?, range: \"A1:C3\"} → {cleared}", has_doc, clear_links),
    ]
}

/// One `links` entry: an object with a non-blank `locator` and a `file` that is a relative path
/// inside the folder ([`SourceLink::is_relative_path`]), or null for no link.
fn link_param(v: &Json) -> std::result::Result<Option<SourceLink>, String> {
    match v {
        Json::Null => Ok(None),
        Json::Object(o) => {
            let text = |k: &str| o.get(k).and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
            let file = text("file").ok_or("a link needs a `file`")?;
            if !SourceLink::is_relative_path(&file) {
                return Err("a link's `file` must be a relative path inside the folder (no absolute path, drive, share, scheme, `.` or `..` segment, backslash or control character)".into());
            }
            let locator = text("locator").ok_or("a link needs a `locator`")?;
            Ok(Some(SourceLink { file: file.into(), locator }))
        }
        _ => Err("each link is an object or null".into()),
    }
}

fn set_links(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    if str_param(p, "range").is_none() {
        return Err(bad("source.set", "missing `range`"));
    }
    let range = target_range(s, p)?;
    let Some(rows) = p.get("links").and_then(Json::as_array) else {
        return Err(bad("source.set", "missing `links` (rows of links or null)"));
    };
    if range.count() > MAX_CELLS {
        return Err(bad("source.set", "range too large"));
    }
    if rows.len() != range.height() as usize {
        return Err(bad("source.set", format!("`links` has {} rows; the range has {}", rows.len(), range.height())));
    }
    let mut parsed: Vec<(CellRef, Option<SourceLink>)> = Vec::with_capacity(range.count() as usize);
    for (ri, row) in rows.iter().enumerate() {
        let Some(row) = row.as_array() else { return Err(bad("source.set", format!("row {} of `links` is not an array", ri + 1))) };
        if row.len() != range.width() as usize {
            return Err(bad("source.set", format!("row {} of `links` has {} entries; the range is {} wide", ri + 1, row.len(), range.width())));
        }
        for (ci, v) in row.iter().enumerate() {
            let Some(at) = range.start.offset(ri as i64, ci as i64) else { continue };
            let link = link_param(v).map_err(|m| bad("source.set", format!("{}: {m}", at.a1())))?;
            parsed.push((at, link));
        }
    }
    // A formula has no source of its own (its inputs do), and a link on one would be read as typed
    // over by any edit that rewrites its references, so only values take links.
    if let Some(sh) = s.doc()?.wb.sheet(sheet)
        && let Some((at, _)) = parsed.iter().find(|(at, l)| l.is_some() && sh.cell(*at).is_some_and(|c| c.formula.is_some()))
    {
        return Err(bad("source.set", format!("{}: a formula has no source of its own; link the values it uses", at.a1())));
    }
    SourceLink::share_files(parsed.iter_mut().filter_map(|(_, l)| l.as_mut()));
    let linked = parsed.iter().filter(|(_, l)| l.is_some()).count();
    edit(s, |cx| {
        let mut set = Vec::new();
        let sh = cx.sheet_mut(sheet)?;
        for (at, link) in parsed {
            match link {
                Some(l) => {
                    sh.sources_mut().insert(at, l);
                    set.push(at);
                }
                None => {
                    sh.remove_sources_in(RangeRef::cell(at), |_| true);
                }
            }
        }
        // A link this command set is new at its address, whatever stood there before.
        cx.note_sources_moved(sheet, set);
        Ok(json!({"linked": linked}))
    })
}

fn get_link(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let at = match cell_param_on(s, p, "cell", sheet)? {
        Some(c) => c,
        None => s.doc()?.selection.active,
    };
    let link = s.doc()?.wb.sheet(sheet).and_then(|sh| sh.sources.get(&at)).cloned();
    Ok(json!({"link": link}))
}

fn clear_links(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    // Never falls back to the selection: a host that forgets the range must not wipe links.
    if str_param(p, "range").is_none() {
        return Err(bad("source.clear", "missing `range`"));
    }
    let ranges = target_ranges(s, p)?;
    edit(s, |cx| {
        let sh = cx.sheet_mut(sheet)?;
        let cleared: usize = ranges.iter().map(|r| sh.remove_sources_in(*r, |_| true)).sum();
        Ok(json!({"cleared": cleared}))
    })
}

#[cfg(test)]
mod tests {
    use gridcraft_core::CellRef;
    use serde_json::{Value as Json, json};

    use crate::Session;

    fn s() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn link_json(locator: &str) -> Json {
        json!({"file": "Cash/Statement.pdf", "locator": locator})
    }

    /// Writes a number and its link on the active sheet, the way the host links a value it wrote.
    fn put(s: &mut Session, cell: &str, value: f64, locator: &str) {
        s.execute("range.setValues", json!({"range": cell, "values": [[value]]})).unwrap();
        s.execute("source.set", json!({"range": cell, "links": [[link_json(locator)]]})).unwrap();
    }

    fn locator_on(s: &Session, sheet: &str, cell: &str) -> Option<String> {
        let wb = &s.doc().unwrap().wb;
        wb.sheet(wb.sheet_index(sheet).unwrap()).unwrap().sources.get(&CellRef::parse(cell).unwrap()).map(|l| l.locator.clone())
    }

    fn locator(s: &Session, cell: &str) -> Option<String> {
        locator_on(s, "Sheet1", cell)
    }

    fn link_count(s: &Session, sheet: &str) -> usize {
        let wb = &s.doc().unwrap().wb;
        wb.sheet(wb.sheet_index(sheet).unwrap()).unwrap().sources.len()
    }

    /// 100,000 links on `A1:J10000`, every one to the same document, each on its own number.
    fn many_links(s: &mut Session) {
        let values: Vec<Vec<f64>> = (0..10_000u32).map(|r| (0..10).map(|c| f64::from(r * 10 + c)).collect()).collect();
        s.execute("range.setValues", json!({"range": "A1:J10000", "values": values})).unwrap();
        let links: Vec<Vec<Json>> = (0..10_000u32)
            .map(|r| {
                (0..10).map(|c| json!({"file": "Cash/Bank Statement September 2019.xlsx", "locator": format!("Sheet1!R{}", r * 10 + c)})).collect()
            })
            .collect();
        s.execute("source.set", json!({"range": "A1:J10000", "links": links})).unwrap();
        assert_eq!(link_count(s, "Sheet1"), 100_000);
    }

    #[test]
    fn an_unrelated_edit_does_not_copy_the_links() {
        let mut s = s();
        many_links(&mut s);
        let undo_before = s.doc().unwrap().undo.len();
        #[allow(clippy::disallowed_methods)]
        let t0 = std::time::Instant::now();
        for i in 0..50 {
            s.execute("cell.set", json!({"cell": "Z1", "input": format!("{i}")})).unwrap();
        }
        #[allow(clippy::disallowed_methods)]
        let took = t0.elapsed();
        let d = s.doc().unwrap();
        // Every snapshot an unrelated edit left on the undo stack shares the one map of links.
        let now = &d.wb.sheets[0].sources;
        for entry in &d.undo[undo_before..] {
            assert!(std::sync::Arc::ptr_eq(&entry.wb.sheets[0].sources, now), "an undo snapshot holds its own copy of the links");
        }
        // Links to one document share its name too.
        let files: Vec<_> = now.values().take(2).map(|l| l.file.clone()).collect();
        assert!(std::sync::Arc::ptr_eq(&files[0], &files[1]), "each link holds its own copy of the file name");
        // Generous: an edit that copied 100,000 links took tens of milliseconds even in release.
        assert!(took < std::time::Duration::from_millis(250), "50 unrelated edits took {took:?}");
    }

    #[test]
    fn a_formula_cell_cannot_be_linked() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A10", "values": [[5]]})).unwrap();
        s.execute("cell.set", json!({"cell": "A1", "input": "=A10"})).unwrap();
        // A formula has no source of its own: its inputs do.
        let err = s.execute("source.set", json!({"range": "A1", "links": [[link_json("page:1")]]})).unwrap_err().to_string();
        assert!(err.contains("A1") && err.contains("formula"), "{err}");
        assert_eq!(locator(&s, "A1"), None);
        // Steve's case: a row inserted above the formula's input rewrites it to =A11. With no
        // link on the formula there is none to lose.
        s.execute("home.insertRows", json!({"rows": "5:5"})).unwrap();
        assert_eq!(link_count(&s, "Sheet1"), 0);
        // A batch naming a formula among numbers is refused whole and changes nothing.
        s.execute("range.setValues", json!({"range": "B1", "values": [[1, 2]]})).unwrap();
        let mixed = json!({"range": "A1:C1", "links": [[link_json("a"), link_json("b"), link_json("c")]]});
        let err = s.execute("source.set", mixed).unwrap_err().to_string();
        assert!(err.contains("A1"), "{err}");
        assert_eq!(link_count(&s, "Sheet1"), 0);
        // An array formula's anchor too.
        s.execute("cell.set", json!({"cell": "E1", "input": "=SEQUENCE(2)"})).unwrap();
        assert!(s.execute("source.set", json!({"range": "E1", "links": [[link_json("e")]]})).is_err());
        // Removing a link from a formula cell (null) is still allowed, and the numbers link.
        s.execute("source.set", json!({"range": "A1:C1", "links": [[null, link_json("b"), link_json("c")]]})).unwrap();
        assert_eq!(locator(&s, "B1").as_deref(), Some("b"));
        // A linked number later typed over with a formula loses its link (a type-over).
        s.execute("cell.set", json!({"cell": "B1", "input": "=C1"})).unwrap();
        assert_eq!(locator(&s, "B1"), None);
        assert_eq!(locator(&s, "C1").as_deref(), Some("c"));
    }

    #[test]
    fn a_link_must_name_a_relative_path_inside_the_folder() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2]]})).unwrap();
        for file in [
            "\\\\attacker\\share\\x.pdf",
            "//attacker/share/x.pdf",
            "/etc/passwd",
            "C:/Windows/win.ini",
            "../x.pdf",
            "Cash/../../x.pdf",
            "Cash\\x.pdf",
            "file:///etc/passwd",
            "https://example.com/x.pdf",
            "a\u{0}b.pdf",
        ] {
            let r = s.execute(
                "source.set",
                json!({"range": "A1:B1", "links": [[{"file": "Cash/Statement.pdf", "locator": "page:1"}, {"file": file, "locator": "page:1"}]]}),
            );
            let err = r.expect_err(file).to_string();
            assert!(err.contains("B1") && err.contains("relative path"), "{file}: {err}");
            assert_eq!(link_count(&s, "Sheet1"), 0, "{file}: a refused batch changes nothing");
        }
        s.execute("source.set", json!({"range": "A1", "links": [[{"file": "Cash/Bank Statement.pdf", "locator": "page:1"}]]})).unwrap();
        assert_eq!(locator(&s, "A1").as_deref(), Some("page:1"));
    }

    #[test]
    fn set_get_and_clear_links_with_undo() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B2", "values": [[10, 20], [30, 40]]})).unwrap();
        let r = s
            .execute("source.set", json!({"sheet": "Sheet1", "range": "B2:C3", "links": [[link_json("a"), null], [link_json("c"), link_json("d")]]}))
            .unwrap();
        assert_eq!(r["linked"], 3);
        let got = s.execute("source.get", json!({"cell": "B2"})).unwrap();
        assert_eq!(got["link"]["locator"], "a");
        assert_eq!(got["link"]["file"], "Cash/Statement.pdf");
        assert!(s.execute("source.get", json!({"cell": "C2"})).unwrap()["link"].is_null());
        // A shape that doesn't match the range is refused and changes nothing.
        assert!(s.execute("source.set", json!({"range": "B2:C3", "links": [[null, null]]})).is_err());
        assert!(s.execute("source.set", json!({"range": "B2", "links": [[{"locator": "no file"}]]})).is_err());
        assert_eq!(link_count(&s, "Sheet1"), 3);
        s.execute("source.clear", json!({"range": "C3"})).unwrap();
        assert_eq!(locator(&s, "C3"), None);
        assert_eq!(locator(&s, "B3").as_deref(), Some("c"));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "C3").as_deref(), Some("d"));
        // null removes one link.
        s.execute("source.set", json!({"range": "B2", "links": [[null]]})).unwrap();
        assert_eq!(locator(&s, "B2"), None);
        // Another sheet, by name.
        s.execute("home.insertSheet", json!({"name": "Other"})).unwrap();
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "D4", "links": [[link_json("z")]]})).unwrap();
        assert_eq!(locator_on(&s, "Sheet1", "D4").as_deref(), Some("z"));
        assert_eq!(link_count(&s, "Other"), 0);
    }

    #[test]
    fn source_commands_survive_bad_params() {
        let mut s = s();
        for id in ["source.set", "source.get", "source.clear"] {
            for params in [
                json!({}),
                json!([1, 2]),
                json!(5),
                json!({"range": "ZZZ999999999", "cell": "", "sheet": 99}),
                json!({"range": "A1:B2", "links": [[null]]}),
                json!({"range": "A1", "links": [[{"file": 3, "locator": []}]]}),
                json!({"range": "A1", "links": "x"}),
                json!({"range": "A1:XFD1048576", "links": []}),
                json!({"cell": "Nope!A1"}),
            ] {
                let r = s.execute(id, params.clone());
                assert!(!matches!(r, Err(crate::EngineError::Internal(..))), "{id} {params}: {r:?}");
            }
        }
    }

    #[test]
    fn clear_and_set_require_an_explicit_range() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "a");
        assert!(s.execute("source.clear", json!({"sheet": "Sheet1"})).is_err());
        assert!(s.execute("source.clear", json!({"sheet": "Sheet1", "range": 5})).is_err());
        assert!(s.execute("source.clear", json!({"sheet": "Sheet1", "range": "not a range"})).is_err());
        assert!(s.execute("source.set", json!({"sheet": "Sheet1", "links": [[null]]})).is_err());
        assert_eq!(locator(&s, "B2").as_deref(), Some("a"));
        assert_eq!(link_count(&s, "Sheet1"), 1);
    }

    #[test]
    fn links_follow_inserted_and_deleted_rows_and_columns() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        s.execute("home.insertRows", json!({"rows": "1:1"})).unwrap();
        assert_eq!(locator(&s, "B3").as_deref(), Some("b2"));
        assert_eq!(locator(&s, "B2"), None);
        s.execute("home.insertColumns", json!({"cols": "A:A"})).unwrap();
        assert_eq!(locator(&s, "C3").as_deref(), Some("b2"));
        s.execute("home.deleteRows", json!({"rows": "1:1"})).unwrap();
        s.execute("home.deleteColumns", json!({"cols": "A:A"})).unwrap();
        assert_eq!(locator(&s, "B2").as_deref(), Some("b2"));
        s.execute("home.deleteRows", json!({"rows": "2:2"})).unwrap();
        assert_eq!(link_count(&s, "Sheet1"), 0, "a deleted row takes its links");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "B2").as_deref(), Some("b2"));
    }

    #[test]
    fn links_follow_inserted_and_deleted_cells_and_moved_columns() {
        let mut s = s();
        put(&mut s, "B2", 2.0, "b2");
        put(&mut s, "B4", 4.0, "b4");
        s.execute("home.insertCells", json!({"range": "B2", "shift": "down"})).unwrap();
        assert_eq!(locator(&s, "B3").as_deref(), Some("b2"));
        assert_eq!(locator(&s, "B5").as_deref(), Some("b4"));
        assert_eq!(locator(&s, "B2"), None);
        s.execute("home.deleteCells", json!({"range": "B3", "shift": "up"})).unwrap();
        assert_eq!(locator(&s, "B3"), None);
        assert_eq!(locator(&s, "B4").as_deref(), Some("b4"));
        assert_eq!(link_count(&s, "Sheet1"), 1, "the deleted cell's link went with it");
        s.execute("home.insertCells", json!({"range": "A4", "shift": "right"})).unwrap();
        assert_eq!(locator(&s, "C4").as_deref(), Some("b4"));
        // Move Columns: C becomes A.
        s.execute("sheet.moveColumns", json!({"cols": "C:C", "before": 0})).unwrap();
        assert_eq!(locator(&s, "A4").as_deref(), Some("b4"));
        assert_eq!(link_count(&s, "Sheet1"), 1);
    }

    #[test]
    fn links_move_down_when_a_headerless_table_is_inserted() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        put(&mut s, "A1", 9.0, "outside");
        s.execute("range.setValues", json!({"range": "B3", "values": [[2]]})).unwrap();
        s.execute("insert.table", json!({"range": "B2:B3", "header": false})).unwrap();
        assert_eq!(locator(&s, "B3").as_deref(), Some("b2"), "first data row moved down one row");
        assert_eq!(locator(&s, "B2"), None);
        assert_eq!(locator(&s, "A1").as_deref(), Some("outside"), "a link outside the table's columns stays");
        assert_eq!(link_count(&s, "Sheet1"), 2);
    }

    #[test]
    fn links_move_with_sorted_rows() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Rank", "Amount"], [2, 200], [1, 100]]})).unwrap();
        s.execute("source.set", json!({"range": "B2", "links": [[link_json("two hundred")]]})).unwrap();
        s.execute("data.sort", json!({"range": "A1:B3", "header": true, "keys": [{"column": "A", "order": "asc"}]})).unwrap();
        assert_eq!(locator(&s, "B2"), None, "the unlinked row moved up");
        assert_eq!(locator(&s, "B3").as_deref(), Some("two hundred"));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "B2").as_deref(), Some("two hundred"));
    }

    #[test]
    fn remove_duplicates_keeps_the_surviving_rows_links() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Amount"], [5], [5], [7]]})).unwrap();
        s.execute("source.set", json!({"range": "A2:A4", "links": [[link_json("first")], [link_json("second")], [link_json("third")]]})).unwrap();
        s.execute("data.removeDuplicates", json!({"range": "A1:A4", "header": true})).unwrap();
        assert_eq!(locator(&s, "A2").as_deref(), Some("first"));
        assert_eq!(locator(&s, "A3").as_deref(), Some("third"));
        assert_eq!(locator(&s, "A4"), None);
        assert_eq!(link_count(&s, "Sheet1"), 2);
    }

    // Figures that share a page or a document locator carry *identical* links, which is the normal
    // case for document sources. A link moved onto an address that held an identical one must not
    // read as the cell having been typed over, so none of these edits may drop one.

    /// The locator on each of `cells`, for an assertion that names them all at once.
    fn locators(s: &Session, cells: &[&str]) -> Vec<Option<String>> {
        cells.iter().map(|c| locator(s, c)).collect()
    }

    /// The number in each of `cells`, to check each link still sits on the figure it describes.
    fn numbers(s: &Session, cells: &[&str]) -> Vec<Option<f64>> {
        let wb = &s.doc().unwrap().wb;
        cells
            .iter()
            .map(|c| match wb.active().unwrap().value(CellRef::parse(c).unwrap()) {
                gridcraft_core::Value::Number(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    /// `A1:A5` holding 1…5, every one linked to the same page of the same document.
    fn same_page() -> Session {
        let mut s = s();
        for i in 1..=5 {
            put(&mut s, &format!("A{i}"), f64::from(i), "page:3");
        }
        assert_eq!(link_count(&s, "Sheet1"), 5);
        s
    }

    fn all_page3(n: usize) -> Vec<Option<String>> {
        vec![Some("page:3".to_string()); n]
    }

    #[test]
    fn identical_links_follow_a_deleted_row() {
        let mut s = same_page();
        s.execute("home.deleteRows", json!({"rows": "2:2"})).unwrap();
        // Row 2's link went with the row; the four below moved up, values and links together.
        assert_eq!(numbers(&s, &["A1", "A2", "A3", "A4"]), [Some(1.0), Some(3.0), Some(4.0), Some(5.0)]);
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4"]), all_page3(4));
        assert_eq!(locator(&s, "A5"), None);
        assert_eq!(link_count(&s, "Sheet1"), 4);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5));
    }

    #[test]
    fn identical_links_follow_an_inserted_row() {
        let mut s = same_page();
        s.execute("home.insertRows", json!({"rows": "2:2"})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A2", "A3"]), [Some(1.0), None, Some(2.0)]);
        assert_eq!(locators(&s, &["A1", "A3", "A4", "A5", "A6"]), all_page3(5));
        assert_eq!(locator(&s, "A2"), None, "the blank row has no link");
        assert_eq!(link_count(&s, "Sheet1"), 5);
    }

    #[test]
    fn identical_links_follow_inserted_and_deleted_cells() {
        let mut s = same_page();
        s.execute("home.insertCells", json!({"range": "A2", "shift": "down"})).unwrap();
        assert_eq!(locators(&s, &["A1", "A3", "A4", "A5", "A6"]), all_page3(5));
        assert_eq!(link_count(&s, "Sheet1"), 5);
        s.execute("home.deleteCells", json!({"range": "A2", "shift": "up"})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A2", "A3", "A4", "A5"]), [Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)]);
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5));
        assert_eq!(link_count(&s, "Sheet1"), 5);
    }

    #[test]
    fn identical_links_follow_sorted_rows() {
        let mut s = same_page();
        s.execute("data.sort", json!({"range": "A1:A5", "keys": [{"column": "A", "order": "desc"}]})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A2", "A3", "A4", "A5"]), [Some(5.0), Some(4.0), Some(3.0), Some(2.0), Some(1.0)]);
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5), "every reordered figure keeps its link");
        assert_eq!(link_count(&s, "Sheet1"), 5);
    }

    #[test]
    fn identical_links_survive_cut_and_paste_and_paste_over() {
        let mut s = same_page();
        // Cut A2:A3 and drop them one row down, over A4:A5, which hold identical links.
        s.execute("edit.cut", json!({"range": "A2:A3"})).unwrap();
        s.execute("edit.paste", json!({"at": "A4"})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A4", "A5"]), [Some(1.0), Some(2.0), Some(3.0)]);
        assert_eq!(locators(&s, &["A1", "A4", "A5"]), all_page3(3));
        assert_eq!(locators(&s, &["A2", "A3"]), [None, None], "the cut cells keep no link");
        assert_eq!(link_count(&s, "Sheet1"), 3);
        // Copy a linked cell over another cell linked to the same page.
        let mut s = same_page();
        s.execute("edit.copy", json!({"range": "A1"})).unwrap();
        s.execute("edit.paste", json!({"at": "A5"})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A5"]), [Some(1.0), Some(1.0)]);
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5), "the pasted link replaces the identical one");
        assert_eq!(link_count(&s, "Sheet1"), 5);
    }

    #[test]
    fn typing_over_one_of_several_identical_links_drops_only_that_one() {
        let mut s = same_page();
        s.execute("range.setValues", json!({"range": "A3", "values": [[99]]})).unwrap();
        assert_eq!(locator(&s, "A3"), None, "the cell typed over loses its link");
        assert_eq!(locators(&s, &["A1", "A2", "A4", "A5"]), all_page3(4), "its neighbours keep theirs");
        assert_eq!(link_count(&s, "Sheet1"), 4);
    }

    #[test]
    fn an_insert_elsewhere_still_drops_a_link_typed_over_in_the_same_edit() {
        let mut s = same_page();
        // Text to Columns inserts a column and rewrites the split cell in one commit: the shift
        // must not excuse the cell it overwrote.
        put_text(&mut s, "C1", "7 8", "split");
        s.execute("data.textToColumns", json!({"range": "C1", "delimiters": [" "]})).unwrap();
        assert_eq!(locator(&s, "C1"), None, "the split cell was typed over");
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5), "the untouched links stay");
    }

    #[test]
    fn a_protected_sheet_refuses_setting_and_clearing_links_on_its_locked_cells() {
        let mut s = s();
        put(&mut s, "A1", 1.0, "locked");
        put(&mut s, "B1", 2.0, "unlocked");
        // B1 is left editable, the way a form leaves its input cells.
        s.execute("home.lockCell", json!({"range": "B1", "on": false})).unwrap();
        s.execute("review.protectSheet", json!({})).unwrap();
        // A link says where a locked cell's value came from, so it's as protected as the value.
        assert!(s.execute("source.set", json!({"range": "A1", "links": [[link_json("new")]]})).is_err());
        assert!(s.execute("source.clear", json!({"range": "A1"})).is_err());
        assert!(s.execute("edit.clearSources", json!({"range": "A1"})).is_err());
        assert_eq!(locator(&s, "A1").as_deref(), Some("locked"), "the refused commands changed nothing");
        // An unlocked cell takes links as it takes values.
        s.execute("source.set", json!({"range": "B1", "links": [[link_json("edited")]]})).unwrap();
        assert_eq!(locator(&s, "B1").as_deref(), Some("edited"));
        s.execute("source.clear", json!({"range": "B1"})).unwrap();
        assert_eq!(locator(&s, "B1"), None);
        // Unprotecting gives the locked cell's link back to the host.
        s.execute("review.unprotectSheet", json!({})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link_json("new")]]})).unwrap();
        assert_eq!(locator(&s, "A1").as_deref(), Some("new"));
    }

    #[test]
    fn a_protected_sheet_still_lets_links_follow_rows_it_allows_to_move() {
        let mut s = same_page();
        s.execute("review.protectSheet", json!({"insertRows": true, "deleteRows": true})).unwrap();
        // An insert the protection allows shifts the links with their cells, protected or not.
        s.execute("home.insertRows", json!({"rows": "2:2"})).unwrap();
        assert_eq!(locators(&s, &["A1", "A3", "A4", "A5", "A6"]), all_page3(5));
        // Deleting a row of locked cells stays refused (as in Excel), so no link is lost that way.
        assert!(s.execute("home.deleteRows", json!({"rows": "3:3"})).is_err());
        assert_eq!(locators(&s, &["A1", "A3", "A4", "A5", "A6"]), all_page3(5));
        // Unprotected, the delete goes through and the links below move up with their values.
        s.execute("review.unprotectSheet", json!({})).unwrap();
        s.execute("home.deleteRows", json!({"rows": "2:2"})).unwrap();
        assert_eq!(numbers(&s, &["A1", "A2", "A3", "A4", "A5"]), [Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)]);
        assert_eq!(locators(&s, &["A1", "A2", "A3", "A4", "A5"]), all_page3(5));
    }

    /// Writes a text value and its link on the active sheet.
    fn put_text(s: &mut Session, cell: &str, text: &str, locator: &str) {
        s.execute("range.setValues", json!({"range": cell, "values": [[text]]})).unwrap();
        s.execute("source.set", json!({"range": cell, "links": [[link_json(locator)]]})).unwrap();
    }

    #[test]
    fn copy_paste_copies_links_and_cut_paste_moves_them() {
        let mut s = s();
        put(&mut s, "B2", 5.0, "five");
        put(&mut s, "D2", 9.0, "nine");
        s.execute("edit.copy", json!({"range": "B2"})).unwrap();
        s.execute("edit.paste", json!({"at": "C5"})).unwrap();
        assert_eq!(locator(&s, "C5").as_deref(), Some("five"));
        assert_eq!(locator(&s, "B2").as_deref(), Some("five"), "a copy leaves the original");
        // Formats leave the destination's own link; values bring the copied one; a combined value has none.
        s.execute("edit.pasteSpecial", json!({"what": "formats", "at": "D2"})).unwrap();
        assert_eq!(locator(&s, "D2").as_deref(), Some("nine"));
        s.execute("edit.pasteSpecial", json!({"what": "values", "at": "D2"})).unwrap();
        assert_eq!(locator(&s, "D2").as_deref(), Some("five"));
        s.execute("edit.pasteSpecial", json!({"what": "values", "operation": "add", "at": "D2"})).unwrap();
        assert_eq!(locator(&s, "D2"), None);
        // Pasting a cell without a link over a linked one removes the link.
        s.execute("edit.copy", json!({"range": "A1"})).unwrap();
        s.execute("edit.paste", json!({"at": "C5"})).unwrap();
        assert_eq!(locator(&s, "C5"), None);
        // Cut and paste (what dragging a selection does) moves the link: none stays behind.
        s.execute("edit.cut", json!({"range": "B2"})).unwrap();
        s.execute("edit.paste", json!({"at": "B3"})).unwrap();
        assert_eq!(locator(&s, "B3").as_deref(), Some("five"));
        assert_eq!(locator(&s, "B2"), None);
        // … also to another sheet.
        s.execute("edit.cut", json!({"range": "B3"})).unwrap();
        s.execute("home.insertSheet", json!({"name": "Dest"})).unwrap();
        s.execute("edit.paste", json!({"at": "E4"})).unwrap();
        assert_eq!(locator_on(&s, "Dest", "E4").as_deref(), Some("five"));
        assert_eq!(locator(&s, "B3"), None);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "B3").as_deref(), Some("five"));
    }

    #[test]
    fn multi_area_copy_keeps_links() {
        let mut s = s();
        put(&mut s, "A1", 1.0, "one");
        put(&mut s, "C1", 3.0, "three");
        s.execute("edit.copy", json!({"range": "A1,C1"})).unwrap();
        s.execute("edit.paste", json!({"at": "A5"})).unwrap();
        assert_eq!(locator(&s, "A5").as_deref(), Some("one"));
        assert_eq!(locator(&s, "B5").as_deref(), Some("three"));
    }

    #[test]
    fn clearing_contents_drops_links_but_clearing_formats_keeps_them() {
        let mut s = s();
        for (c, l) in [("B2", "b2"), ("C2", "c2"), ("D2", "d2")] {
            put(&mut s, c, 1.0, l);
        }
        s.execute("edit.clearFormats", json!({"range": "B2"})).unwrap();
        assert_eq!(locator(&s, "B2").as_deref(), Some("b2"));
        s.execute("edit.clearContents", json!({"range": "B2"})).unwrap();
        assert_eq!(locator(&s, "B2"), None);
        s.execute("edit.clearAll", json!({"range": "C2"})).unwrap();
        assert_eq!(locator(&s, "C2"), None);
        assert_eq!(locator(&s, "D2").as_deref(), Some("d2"));
    }

    #[test]
    fn clear_sources_removes_only_the_links() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        put(&mut s, "C2", 2.0, "c2");
        // A link on a cell with no value, outside the used range, is still in the range.
        s.execute("source.set", json!({"range": "H9", "links": [[link_json("h9")]]})).unwrap();
        s.execute("edit.clearSources", json!({"range": "B2:H9"})).unwrap();
        assert_eq!(link_count(&s, "Sheet1"), 0);
        let v = |s: &Session, c: &str| s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(c).unwrap());
        assert_eq!(v(&s, "B2"), gridcraft_core::Value::Number(1.0), "the values stay");
        assert_eq!(v(&s, "C2"), gridcraft_core::Value::Number(2.0));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "H9").as_deref(), Some("h9"));
        // With no range it clears the selection, like the other Clear commands.
        s.execute("selection.set", json!({"range": "C2"})).unwrap();
        s.execute("edit.clearSources", json!({})).unwrap();
        assert_eq!(locator(&s, "C2"), None);
        assert_eq!(locator(&s, "B2").as_deref(), Some("b2"));
        // Clear All reaches a link beyond the used range too.
        s.execute("edit.clearAll", json!({"range": "H9"})).unwrap();
        assert_eq!(locator(&s, "H9"), None);
    }

    #[test]
    fn typing_a_different_value_drops_the_link_and_reentering_it_keeps_it() {
        let mut s = s();
        put(&mut s, "A1", 1.0, "a1");
        put(&mut s, "A2", 2.0, "a2");
        put_text(&mut s, "A3", "cash", "a3");
        // F2 then Enter on an unchanged value.
        s.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        assert_eq!(locator(&s, "A1").as_deref(), Some("a1"), "re-entering the same value keeps the link");
        s.execute("range.setValues", json!({"range": "A3", "values": [["cash"]]})).unwrap();
        assert_eq!(locator(&s, "A3").as_deref(), Some("a3"));
        s.execute("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
        assert_eq!(locator(&s, "A1"), None, "a typed value is not the sourced one");
        s.execute("cell.set", json!({"cell": "A2", "input": "=1+1"})).unwrap();
        assert_eq!(locator(&s, "A2"), None, "a formula over a linked value, even an equal one");
        s.execute("range.fill", json!({"range": "A3", "input": "bank"})).unwrap();
        assert_eq!(locator(&s, "A3"), None, "range.fill (Ctrl+Enter)");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "A3").as_deref(), Some("a3"), "undo brings the link back");
        // Pasted text and Fill Down over a linked cell.
        s.execute("edit.paste", json!({"at": "A3", "text": "5"})).unwrap();
        assert_eq!(locator(&s, "A3"), None, "pasted text");
        put(&mut s, "B1", 7.0, "b1");
        put(&mut s, "B2", 8.0, "b2");
        s.execute("edit.fillDown", json!({"range": "B1:B2"})).unwrap();
        assert_eq!(locator(&s, "B1").as_deref(), Some("b1"), "the fill source keeps its link");
        assert_eq!(locator(&s, "B2"), None, "a filled cell neither keeps its own link nor copies one");
    }

    #[test]
    fn autosum_text_to_columns_and_replace_over_a_linked_cell_drop_its_link() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1], [2]]})).unwrap();
        put(&mut s, "A3", 9.0, "a3");
        // A block AutoSum writes its totals below the block, over A3.
        s.execute("formulas.autoSum", json!({"range": "A1:A2"})).unwrap();
        assert_eq!(locator(&s, "A3"), None, "AutoSum");
        put_text(&mut s, "C1", "x,y", "c1");
        s.execute("data.textToColumns", json!({"range": "C1", "delimiters": [","]})).unwrap();
        assert_eq!(locator(&s, "C1"), None, "Text to Columns");
        put(&mut s, "E1", 1.0, "e1");
        put(&mut s, "E2", 31.0, "e2");
        put(&mut s, "E3", 2.0, "e3");
        s.execute("edit.replace", json!({"what": "1", "with": "6", "all": true})).unwrap();
        assert_eq!(locator(&s, "E1"), None, "Replace");
        assert_eq!(locator(&s, "E2"), None);
        assert_eq!(locator(&s, "E3").as_deref(), Some("e3"), "a cell Replace didn't change keeps its link");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "E1").as_deref(), Some("e1"));
        assert_eq!(locator(&s, "E2").as_deref(), Some("e2"));
    }

    #[test]
    fn writing_a_value_then_linking_it_keeps_the_new_link() {
        // The host's batch: write the value, then its link, as two commands in a row.
        let mut s = s();
        put(&mut s, "C1", 5.0, "old");
        s.execute("range.setValues", json!({"range": "C1", "values": [[100]]})).unwrap();
        assert_eq!(locator(&s, "C1"), None);
        s.execute("source.set", json!({"range": "C1", "links": [[link_json("c1")]]})).unwrap();
        assert_eq!(locator(&s, "C1").as_deref(), Some("c1"));
        // Linking first and writing after drops it: the value isn't the sourced one.
        s.execute("source.set", json!({"range": "C2", "links": [[link_json("c2")]]})).unwrap();
        s.execute("range.setValues", json!({"range": "C2", "values": [[3]]})).unwrap();
        assert_eq!(locator(&s, "C2"), None);
    }

    #[test]
    fn merging_drops_the_links_of_the_cells_it_empties() {
        let mut s = s();
        for (c, l) in [("A1", "a1"), ("B1", "b1"), ("A3", "a3"), ("B3", "b3"), ("A4", "a4"), ("B4", "b4"), ("D1", "d1"), ("E1", "e1")] {
            put(&mut s, c, 1.0, l);
        }
        s.execute("home.mergeCells", json!({"range": "A1:B1"})).unwrap();
        assert_eq!(locator(&s, "A1").as_deref(), Some("a1"), "the anchor keeps its link");
        assert_eq!(locator(&s, "B1"), None);
        s.execute("home.mergeAcross", json!({"range": "A3:B4"})).unwrap();
        assert_eq!(locator(&s, "A3").as_deref(), Some("a3"), "each row's anchor keeps its link");
        assert_eq!(locator(&s, "A4").as_deref(), Some("a4"));
        assert_eq!(locator(&s, "B3"), None);
        assert_eq!(locator(&s, "B4"), None);
        s.execute("home.mergeCenter", json!({"range": "D1:E1"})).unwrap();
        assert_eq!(locator(&s, "D1").as_deref(), Some("d1"));
        assert_eq!(locator(&s, "E1"), None);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(locator(&s, "E1").as_deref(), Some("e1"), "undo brings the link back");
        // Unmerging empties nothing, so it drops nothing.
        s.execute("home.mergeCenter", json!({"range": "A1:B1"})).unwrap();
        assert_eq!(locator(&s, "A1").as_deref(), Some("a1"));
        // A link on a cell with no value inside the merged area goes too.
        put(&mut s, "F1", 1.0, "f1");
        s.execute("source.set", json!({"range": "G1", "links": [[link_json("g1")]]})).unwrap();
        s.execute("home.mergeCells", json!({"range": "F1:G1"})).unwrap();
        assert_eq!(locator(&s, "F1").as_deref(), Some("f1"));
        assert_eq!(locator(&s, "G1"), None);
    }

    #[test]
    fn merge_across_keeps_each_rows_anchor_link_over_many_rows_and_columns() {
        let mut s = s();
        for (c, l) in [("B6", "b6"), ("C6", "c6"), ("D6", "d6"), ("B7", "b7"), ("D7", "d7"), ("B8", "b8"), ("E7", "outside"), ("A7", "left")] {
            put(&mut s, c, 1.0, l);
        }
        // A link on a cell with no value, in a row's emptied part.
        s.execute("source.set", json!({"range": "C8", "links": [[link_json("c8")]]})).unwrap();
        s.execute("home.mergeAcross", json!({"range": "B6:D8"})).unwrap();
        for (c, l) in [("B6", "b6"), ("B7", "b7"), ("B8", "b8"), ("E7", "outside"), ("A7", "left")] {
            assert_eq!(locator(&s, c).as_deref(), Some(l), "{c} keeps its link");
        }
        for c in ["C6", "D6", "D7", "C8"] {
            assert_eq!(locator(&s, c), None, "{c} was emptied by the merge");
        }
        // A single column merges nothing across, so it drops nothing.
        put(&mut s, "G1", 1.0, "g1");
        put(&mut s, "G2", 2.0, "g2");
        s.execute("home.mergeAcross", json!({"range": "G1:G2"})).unwrap();
        assert_eq!(locator(&s, "G2").as_deref(), Some("g2"));
        // Whole columns: one pass over the links, not one per row.
        s.execute("home.mergeAcross", json!({"range": "I:J"})).unwrap();
        assert_eq!(locator(&s, "B6").as_deref(), Some("b6"));
    }
}
