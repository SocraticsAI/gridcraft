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
}
