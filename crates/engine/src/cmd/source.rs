// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links: which document each hard-coded value came from (see
//! `gridcraft_model::SourceLink`). A host that fills cells from its documents writes the values,
//! then their links (`source.set`); typing over a cell drops its link.

use gridcraft_core::CellRef;
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
            "{sheet?, range: \"A1:C3\", links: [[{file, locator, storeRow} | null]] (one row per range row; null removes a link)} → {linked}",
            has_doc,
            set_links
        ),
        cmd!(query "source.get", "Cell Source", [], None, "{sheet?, cell: \"B2\"} → {link: {file, locator, storeRow} | null}", has_doc, get_link),
        cmd!("source.clear", "Clear Cell Sources", [], None, "{sheet?, range: \"A1:C3\"} → {cleared}", has_doc, clear_links),
        cmd!(
            query "source.trace",
            "Trace Cell Sources",
            [],
            None,
            "{sheet?, cell: \"B2\"} → {links: [{sheet, cell, link}], unsourced: [{sheet, cell}], untraceable, truncated}",
            has_doc,
            trace_links
        ),
    ]
}

/// One `links` entry: an object with a non-blank `file` and `locator` (and optional `storeRow`),
/// or null for no link.
fn link_param(v: &Json) -> std::result::Result<Option<SourceLink>, String> {
    match v {
        Json::Null => Ok(None),
        Json::Object(o) => {
            let text = |k: &str| o.get(k).and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
            let file = text("file").ok_or("a link needs a `file`")?;
            let locator = text("locator").ok_or("a link needs a `locator`")?;
            Ok(Some(SourceLink { file, locator, store_row: text("storeRow").unwrap_or_default() }))
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
    let linked = parsed.iter().filter(|(_, l)| l.is_some()).count();
    edit(s, |cx| {
        let sh = cx.sheet_mut(sheet)?;
        for (at, link) in parsed {
            match link {
                Some(l) => {
                    sh.sources.insert(at, l);
                }
                None => {
                    sh.sources.remove(&at);
                }
            }
        }
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
        let before = sh.sources.len();
        sh.sources.retain(|c, _| !ranges.iter().any(|r| r.contains(*c)));
        Ok(json!({"cleared": before - sh.sources.len()}))
    })
}

fn trace_links(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let at = match cell_param_on(s, p, "cell", sheet)? {
        Some(c) => c,
        None => s.doc()?.selection.active,
    };
    let d = s.doc()?;
    let t = crate::source_state::trace(&d.wb, sheet, at);
    let name = |si: usize| d.wb.sheet(si).map(|sh| sh.name.clone()).unwrap_or_default();
    Ok(json!({
        "links": t.links.iter().map(|l| json!({"sheet": name(l.sheet), "cell": l.cell.a1(), "link": l.link})).collect::<Vec<_>>(),
        "unsourced": t.unsourced.iter().map(|(si, c)| json!({"sheet": name(*si), "cell": c.a1()})).collect::<Vec<_>>(),
        "untraceable": t.untraceable,
        "truncated": t.truncated,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gridcraft_core::CellRef;
    use serde_json::{Value as Json, json};

    use crate::Session;

    fn s() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn link_json(locator: &str) -> Json {
        json!({"file": "Data Room/Cash/Statement.pdf", "locator": locator, "storeRow": format!("row-{locator}")})
    }

    /// Writes a number and its link on the active sheet, the way the host fills from its store.
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
        assert_eq!(got["link"]["storeRow"], "row-a");
        assert_eq!(got["link"]["file"], "Data Room/Cash/Statement.pdf");
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
        for id in ["source.set", "source.get", "source.clear", "source.trace"] {
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

    fn trace(s: &mut Session, params: Json) -> Json {
        s.execute("source.trace", params).unwrap()
    }

    fn cells(v: &Json, key: &str) -> Vec<String> {
        let mut out: Vec<String> =
            v[key].as_array().unwrap().iter().map(|x| format!("{}!{}", x["sheet"].as_str().unwrap(), x["cell"].as_str().unwrap())).collect();
        out.sort();
        out
    }

    #[test]
    fn a_hard_coded_value_traces_to_its_own_link_or_to_nothing() {
        let mut s = s();
        put(&mut s, "B2", 1318609.0, "opening");
        s.execute("range.setValues", json!({"range": "B3", "values": [[7], ["text"]]})).unwrap();
        let t = trace(&mut s, json!({"cell": "B2"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B2"]);
        assert_eq!(t["links"][0]["link"]["locator"], "opening");
        assert_eq!(t["links"][0]["link"]["storeRow"], "row-opening");
        assert_eq!((t["untraceable"].as_bool(), t["truncated"].as_bool()), (Some(false), Some(false)));
        assert_eq!(cells(&trace(&mut s, json!({"cell": "B3"})), "unsourced"), ["Sheet1!B3"]);
        let text = trace(&mut s, json!({"cell": "B4"}));
        assert!(cells(&text, "links").is_empty() && cells(&text, "unsourced").is_empty());
    }

    #[test]
    fn a_formula_traces_across_sheets_and_3d_references() {
        let mut s = s();
        put(&mut s, "B2", 100.0, "jan");
        s.execute("range.setValues", json!({"range": "B3", "values": [[5]]})).unwrap();
        s.execute("home.insertSheet", json!({"name": "Feb"})).unwrap();
        s.execute("range.setValues", json!({"range": "Feb!B2", "values": [[200]]})).unwrap();
        s.execute("source.set", json!({"sheet": "Feb", "range": "B2", "links": [[link_json("feb")]]})).unwrap();
        s.execute("cell.set", json!({"sheet": "Feb", "cell": "C1", "input": "=SUM(Sheet1:Feb!B2)"})).unwrap();
        s.execute("cell.set", json!({"sheet": "Feb", "cell": "C2", "input": "=Sheet1!B2+Sheet1!B3"})).unwrap();
        s.execute("cell.set", json!({"sheet": "Feb", "cell": "C3", "input": "=C1+C2"})).unwrap();
        let t = trace(&mut s, json!({"sheet": "Feb", "cell": "C1"}));
        assert_eq!(cells(&t, "links"), ["Feb!B2", "Sheet1!B2"]);
        let t = trace(&mut s, json!({"sheet": "Feb", "cell": "C2"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B2"]);
        assert_eq!(cells(&t, "unsourced"), ["Sheet1!B3"]);
        // Through formulas, each source once.
        let t = trace(&mut s, json!({"sheet": "Feb", "cell": "C3"}));
        assert_eq!(cells(&t, "links"), ["Feb!B2", "Sheet1!B2"]);
        assert_eq!(cells(&t, "unsourced"), ["Sheet1!B3"]);
    }

    #[test]
    fn defined_names_and_table_references_are_resolved() {
        let mut s = s();
        put(&mut s, "B2", 100.0, "opening");
        s.execute("formulas.defineName", json!({"name": "Opening", "refersTo": "=Sheet1!$B$2"})).unwrap();
        s.execute("formulas.defineName", json!({"name": "Twice", "refersTo": "=Opening*2"})).unwrap();
        s.execute("cell.set", json!({"cell": "E1", "input": "=Opening*2"})).unwrap();
        s.execute("cell.set", json!({"cell": "E2", "input": "=Twice+1"})).unwrap();
        assert_eq!(cells(&trace(&mut s, json!({"cell": "E1"})), "links"), ["Sheet1!B2"]);
        assert_eq!(cells(&trace(&mut s, json!({"cell": "E2"})), "links"), ["Sheet1!B2"]);
        s.execute("range.setValues", json!({"range": "A10", "values": [["Item", "Amount"], ["Rent", 1200], ["Fees", 300]]})).unwrap();
        s.execute("source.set", json!({"range": "B11:B12", "links": [[link_json("rent")], [link_json("fees")]]})).unwrap();
        s.execute("insert.table", json!({"range": "A10:B12", "header": true, "name": "Costs"})).unwrap();
        s.execute("cell.set", json!({"cell": "D10", "input": "=SUM(Costs[Amount])"})).unwrap();
        s.execute("cell.set", json!({"cell": "D11", "input": "=ROWS(Costs)+SUM(Costs)"})).unwrap();
        assert_eq!(cells(&trace(&mut s, json!({"cell": "D10"})), "links"), ["Sheet1!B11", "Sheet1!B12"]);
        assert_eq!(cells(&trace(&mut s, json!({"cell": "D11"})), "links"), ["Sheet1!B11", "Sheet1!B12"]);
    }

    #[test]
    fn a_name_holding_a_this_row_reference_is_expanded_on_each_row_it_is_used() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A10", "values": [["Item", "Amount"], ["Rent", 1200], ["Fees", 300]]})).unwrap();
        s.execute("source.set", json!({"range": "B11:B12", "links": [[link_json("rent")], [link_json("fees")]]})).unwrap();
        s.execute("insert.table", json!({"range": "A10:B12", "header": true, "name": "Costs"})).unwrap();
        s.execute("formulas.defineName", json!({"name": "ThisAmt", "refersTo": "=Costs[@Amount]"})).unwrap();
        s.execute("cell.set", json!({"cell": "E11", "input": "=ThisAmt"})).unwrap();
        s.execute("cell.set", json!({"cell": "E12", "input": "=ThisAmt"})).unwrap();
        s.execute("cell.set", json!({"cell": "F1", "input": "=E11+E12"})).unwrap();
        assert_eq!(cells(&trace(&mut s, json!({"cell": "E12"})), "links"), ["Sheet1!B12"]);
        assert_eq!(cells(&trace(&mut s, json!({"cell": "F1"})), "links"), ["Sheet1!B11", "Sheet1!B12"]);
        let marker = |s: &Session, cell: &str| crate::source_state::source_state(&s.doc().unwrap().wb, 0, CellRef::parse(cell).unwrap());
        // A formula is traced but never marked; the values it reads are.
        assert_eq!(marker(&s, "F1"), None);
        assert_eq!(marker(&s, "B12"), Some(crate::source_state::SourceState::Sourced));
        // With B12 unlinked, F1 rests on an unlinked number: listed, and B12's marker turns amber.
        s.execute("source.clear", json!({"range": "B12"})).unwrap();
        let t = trace(&mut s, json!({"cell": "F1"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B11"]);
        assert_eq!(t["unsourced"], json!([{"sheet": "Sheet1", "cell": "B12"}]));
        assert_eq!(marker(&s, "F1"), None);
        assert_eq!(marker(&s, "B12"), Some(crate::source_state::SourceState::Unsourced));
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn names_that_fan_out_are_expanded_once_each() {
        // N_k = N_k+1 four times over: without a visited set, N_1 would expand 4^15 copies of N_16.
        // (`N1` itself is a cell address, so the names carry an underscore.) The evaluator has no such
        // guard, so `=N_1` goes into the workbook directly, uncalculated: `cell.set` would compute it.
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        for k in 1..16 {
            let next = format!("N_{}", k + 1);
            s.execute("formulas.defineName", json!({"name": format!("N_{k}"), "refersTo": format!("={next}+{next}+{next}+{next}")})).unwrap();
        }
        s.execute("formulas.defineName", json!({"name": "N_16", "refersTo": "=Sheet1!$B$2+Sheet1!$B$3"})).unwrap();
        s.execute("range.setValues", json!({"range": "B3", "values": [[2]]})).unwrap();
        let formula = gridcraft_model::Cell { formula: Some(Arc::new(gridcraft_model::Formula::new("=N_1"))), ..Default::default() };
        let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        wb.sheet_mut(0).unwrap().set_cell(CellRef::parse("E1").unwrap(), formula);
        let started = std::time::Instant::now();
        let t = trace(&mut s, json!({"cell": "E1"}));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_eq!(cells(&t, "links"), ["Sheet1!B2"]);
        assert_eq!(cells(&t, "unsourced"), ["Sheet1!B3"]);
        assert_eq!((t["untraceable"].as_bool(), t["truncated"].as_bool()), (Some(false), Some(false)));
    }

    #[test]
    fn names_nested_past_the_depth_limit_are_untraceable() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        for k in 1..=20 {
            s.execute("formulas.defineName", json!({"name": format!("D_{k}"), "refersTo": format!("=D_{}", k + 1)})).unwrap();
        }
        s.execute("formulas.defineName", json!({"name": "D_21", "refersTo": "=Sheet1!$B$2"})).unwrap();
        s.execute("cell.set", json!({"cell": "E1", "input": "=D_1"})).unwrap();
        let t = trace(&mut s, json!({"cell": "E1"}));
        assert_eq!(t["untraceable"], true);
        assert!(cells(&t, "links").is_empty());
    }

    #[test]
    fn cycles_end_and_dynamic_references_are_untraceable() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        s.execute("cell.set", json!({"cell": "A1", "input": "=B1+B2"})).unwrap();
        s.execute("cell.set", json!({"cell": "B1", "input": "=A1+1"})).unwrap();
        let t = trace(&mut s, json!({"cell": "A1"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B2"]);
        assert_eq!(t["untraceable"], false);
        s.execute("cell.set", json!({"cell": "C1", "input": "=INDIRECT(\"B2\")"})).unwrap();
        s.execute("cell.set", json!({"cell": "C2", "input": "=OFFSET(B2,0,0)"})).unwrap();
        s.execute("cell.set", json!({"cell": "C3", "input": "=C2*2"})).unwrap();
        for c in ["C1", "C2", "C3"] {
            assert_eq!(trace(&mut s, json!({"cell": c}))["untraceable"], true, "{c}");
        }
    }

    #[test]
    fn the_trace_stops_at_its_cap() {
        let mut s = s();
        let rows: Vec<Json> = (0..12_000).map(|i| json!([i])).collect();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        // The first cell read is linked; the last ones are never reached.
        s.execute("source.set", json!({"range": "A1", "links": [[link_json("one")]]})).unwrap();
        s.execute("source.set", json!({"range": "A12000", "links": [[link_json("last")]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=SUM(A1:A12000)"})).unwrap();
        let t = trace(&mut s, json!({"cell": "C1"}));
        assert_eq!(t["truncated"], true);
        assert_eq!(cells(&t, "links"), ["Sheet1!A1"]);
        let seen = t["unsourced"].as_array().unwrap().len();
        // The cap counts every cell visited: the traced formula, the linked A1 and the unlinked rest.
        assert_eq!(1 + 1 + seen, crate::source_state::TRACE_CAP, "{seen} cells listed");
        // Below the cap nothing is cut.
        s.execute("cell.set", json!({"cell": "C2", "input": "=SUM(A1:A100)"})).unwrap();
        let t = trace(&mut s, json!({"cell": "C2"}));
        assert_eq!(t["truncated"], false);
        assert_eq!(t["unsourced"].as_array().unwrap().len(), 99);
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn whole_column_references_over_many_stored_blanks_stay_bounded() {
        // 200,000 stored blank cells in A and B (formatted, never typed in), a linked number below
        // them, and 5,000 formulas each reading column B. Written straight into the workbook:
        // `cell.set` would recalculate every SUM.
        let mut s = s();
        put(&mut s, "B200001", 7.0, "b");
        let at = |a1: &str| CellRef::parse(a1).unwrap();
        let formula = |text: &str| gridcraft_model::Cell { formula: Some(Arc::new(gridcraft_model::Formula::new(text))), ..Default::default() };
        let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        let sh = wb.sheet_mut(0).unwrap();
        let blank = gridcraft_model::Cell { style: gridcraft_model::StyleId(1), ..Default::default() };
        for row in 0..200_000 {
            sh.set_cell(CellRef::new(row, 0), blank.clone());
            sh.set_cell(CellRef::new(row, 1), blank.clone());
        }
        assert_eq!(sh.cells.len(), 400_001);
        for k in 1..=5_000 {
            // The same area over and over: scanned once.
            sh.set_cell(at(&format!("D{k}")), formula(&format!("=SUM(B:B)+D{}", k + 1)));
            // A different area each time: the scan budget ends the trace.
            sh.set_cell(at(&format!("E{k}")), formula(&format!("=SUM(B{k}:B1048576)+E{}", k + 1)));
        }
        let started = std::time::Instant::now();
        let t = trace(&mut s, json!({"cell": "D1"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B200001"]);
        assert_eq!(t["truncated"], false);
        let t = trace(&mut s, json!({"cell": "E1"}));
        assert_eq!(t["truncated"], true);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn a_spilled_value_traces_through_its_formula() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        s.execute("range.setValues", json!({"range": "B3", "values": [[2]]})).unwrap();
        s.execute("cell.set", json!({"cell": "D1", "input": "=B2:B3*1"})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::parse("D2").unwrap()), gridcraft_core::Value::Number(2.0));
        let t = trace(&mut s, json!({"cell": "D2"}));
        assert_eq!(cells(&t, "links"), ["Sheet1!B2"]);
        assert_eq!(cells(&t, "unsourced"), ["Sheet1!B3"]);
    }
}
