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
/// Most links one `source.list` returns (as many as a saved file carries).
const MAX_LISTED: usize = 2_000_000;
/// Most entries one `source.setMissing` may name.
const MAX_MISSING: usize = 1_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "source.set",
            "Set Cell Sources",
            [],
            None,
            "{sheet?, range: \"A1:C3\", links: [[{file, locator, hash?, size?} | null]] (one row per range row; null removes a link; hash: SHA-256 as 64 lowercase hex, size: bytes)} → {linked}",
            has_doc,
            set_links
        ),
        cmd!(query "source.get", "Cell Source", [], None, "{sheet?, cell: \"B2\"} → {link: {file, locator, hash?, size?} | null}", has_doc, get_link),
        cmd!(
            query "source.list",
            "List Cell Sources",
            [],
            None,
            "{} → {links: [{sheet, cell, link}], truncated} (every link in the workbook: sheet order, then row, then column)",
            has_doc,
            list_links
        ),
        // Display state, not an edit: never journaled, undone, saved or marking the workbook dirty.
        cmd!(
            query "source.setMissing",
            "Set Missing Sources",
            [],
            None,
            "{missing: [{sheet, cell: \"B2\"}]} → {missing, skipped} (replaces the set; linked numbers in it draw grey; unknown sheets and bad cells are skipped)",
            has_doc,
            set_missing
        ),
        cmd!("source.clear", "Clear Cell Sources", [], None, "{sheet?, range: \"A1:C3\"} → {cleared}", has_doc, clear_links),
    ]
}

/// One `links` entry: an object with a non-blank `file` and `locator` (and optional `hash` and
/// `size`), or null for no link.
fn link_param(v: &Json) -> std::result::Result<Option<SourceLink>, String> {
    match v {
        Json::Null => Ok(None),
        Json::Object(o) => {
            let text = |k: &str| o.get(k).and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
            let file = text("file").ok_or("a link needs a `file`")?;
            let locator = text("locator").ok_or("a link needs a `locator`")?;
            let hash = match o.get("hash") {
                None | Some(Json::Null) => None,
                Some(Json::String(h)) if SourceLink::is_hash(h) => Some(h.clone()),
                Some(_) => return Err("`hash` is a SHA-256 as 64 lowercase hex digits".into()),
            };
            let size = match o.get("size") {
                None | Some(Json::Null) => None,
                Some(n) => Some(n.as_u64().ok_or("`size` is a whole number of bytes")?),
            };
            Ok(Some(SourceLink { file, locator, hash, size }))
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

fn list_links(s: &mut Session, _p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let mut links = Vec::new();
    let mut truncated = false;
    'sheets: for sh in &d.wb.sheets {
        for (c, l) in &sh.sources {
            if links.len() >= MAX_LISTED {
                truncated = true;
                break 'sheets;
            }
            links.push(json!({"sheet": sh.name, "cell": c.a1(), "link": l}));
        }
    }
    Ok(json!({"links": links, "truncated": truncated}))
}

/// Replaces the document's missing-source set. Entries naming an unknown sheet or a bad cell are
/// skipped (and counted); a list longer than [`MAX_MISSING`] is refused.
fn set_missing(s: &mut Session, p: &Json) -> Result<Json> {
    let Some(entries) = p.get("missing").and_then(Json::as_array) else {
        return Err(bad("source.setMissing", "missing `missing` (a list of {sheet, cell}, empty to clear)"));
    };
    if entries.len() > MAX_MISSING {
        return Err(bad("source.setMissing", format!("at most {MAX_MISSING} entries")));
    }
    let d = s.doc_mut()?;
    let mut set: std::collections::BTreeMap<String, std::collections::BTreeSet<CellRef>> = Default::default();
    let mut skipped = 0usize;
    for e in entries {
        let sheet = e.get("sheet").and_then(Json::as_str).and_then(|n| d.wb.sheet_index(n)).and_then(|i| d.wb.sheet(i));
        let cell = e.get("cell").and_then(Json::as_str).and_then(|a| CellRef::parse(a.trim())).filter(CellRef::is_valid);
        match (sheet, cell) {
            (Some(sh), Some(c)) => {
                set.entry(sh.name.clone()).or_default().insert(c);
            }
            _ => skipped += 1,
        }
    }
    let count: usize = set.values().map(|c| c.len()).sum();
    d.source_missing = set;
    Ok(json!({"missing": count, "skipped": skipped}))
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
        json!({"file": "Data Room/Cash/Statement.pdf", "locator": locator})
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

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    fn fingerprinted(locator: &str) -> Json {
        let mut l = link_json(locator);
        l["hash"] = json!(HASH);
        l["size"] = json!(48_213);
        l
    }

    #[test]
    fn fingerprints_round_trip_through_set_get_and_saved_files() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B2", "values": [[10, 20]]})).unwrap();
        s.execute("source.set", json!({"range": "B2:C2", "links": [[fingerprinted("a"), link_json("b")]]})).unwrap();
        let got = s.execute("source.get", json!({"cell": "B2"})).unwrap();
        assert_eq!((got["link"]["hash"].as_str(), got["link"]["size"].as_u64()), (Some(HASH), Some(48_213)));
        let plain = s.execute("source.get", json!({"cell": "C2"})).unwrap();
        let fields = plain["link"].as_object().unwrap();
        assert!(!fields.contains_key("hash") && !fields.contains_key("size"), "absent stays absent: {plain}");
        let before = s.doc().unwrap().wb.sheet(0).unwrap().sources.clone();
        for name in ["book.json", "book.xlsx"] {
            let bytes = crate::io::save_bytes(&s.doc().unwrap().wb, name).unwrap();
            let (back, _) = crate::io::open_bytes(name, &bytes).unwrap();
            assert_eq!(back.sheet(0).unwrap().sources, before, "{name}");
            let c2 = back.sheet(0).unwrap().sources.get(&CellRef::parse("C2").unwrap()).unwrap();
            assert_eq!((c2.hash.as_deref(), c2.size), (None, None), "{name}");
        }
    }

    #[test]
    fn a_large_load_from_a_few_files_reopens_exactly_from_a_saved_xlsx() {
        let mut s = s();
        s.execute("home.insertSheet", json!({"name": "Q3 \"Café\" & <Co>"})).unwrap();
        // 20,000 cells from one fingerprinted statement.
        let rows: Vec<Json> =
            (0..2_000).map(|r| (0..10).map(|c| fingerprinted(&format!("Page {} / line {}", (r * 10 + c) / 50, r * 10 + c))).collect()).collect();
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "A1:J2000", "links": rows})).unwrap();
        // A few more from other documents, with and without fingerprints.
        let other = json!({"file": "Data Room/a & b <c> \"d\"/GL_x0041_.xlsx", "locator": "GL Detail!C16102 & <x>"});
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "L1:L2", "links": [[other.clone()], [link_json("plain")]]})).unwrap();
        s.execute("source.set", json!({"sheet": "Q3 \"Café\" & <Co>", "range": "B2:C2", "links": [[other, fingerprinted("there")]]})).unwrap();
        let wb = &s.doc().unwrap().wb;
        assert_eq!(wb.sheet(0).unwrap().sources.len(), 20_002);
        let bytes = crate::io::save_bytes(wb, "book.xlsx").unwrap();
        let (back, _) = crate::io::open_bytes("book.xlsx", &bytes).unwrap();
        for i in 0..wb.sheets.len() {
            assert_eq!(back.sheet(i).unwrap().sources, wb.sheet(i).unwrap().sources, "sheet {i}");
        }
        let again = crate::io::save_bytes(&back, "book.xlsx").unwrap();
        let (third, _) = crate::io::open_bytes("book.xlsx", &again).unwrap();
        for i in 0..wb.sheets.len() {
            assert_eq!(third.sheet(i).unwrap().sources, wb.sheet(i).unwrap().sources, "sheet {i}");
        }
    }

    #[test]
    fn bad_fingerprints_are_refused_and_change_nothing() {
        let mut s = s();
        s.execute("source.set", json!({"range": "A1", "links": [[fingerprinted("kept")]]})).unwrap();
        let bad_hashes = [json!(HASH.to_uppercase()), json!(&HASH[1..]), json!(format!("{HASH}0")), json!("g".repeat(64)), json!(12), json!({})];
        for h in bad_hashes {
            let mut l = link_json("x");
            l["hash"] = h.clone();
            assert!(matches!(s.execute("source.set", json!({"range": "A1", "links": [[l]]})), Err(crate::EngineError::BadParams { .. })), "hash {h}");
        }
        for n in [json!(-1), json!(1.5), json!("12"), json!(true)] {
            let mut l = link_json("x");
            l["size"] = n.clone();
            assert!(matches!(s.execute("source.set", json!({"range": "A1", "links": [[l]]})), Err(crate::EngineError::BadParams { .. })), "size {n}");
        }
        assert_eq!(locator(&s, "A1").as_deref(), Some("kept"));
        // null is the same as leaving them out.
        let mut l = link_json("nulls");
        l["hash"] = Json::Null;
        l["size"] = Json::Null;
        s.execute("source.set", json!({"range": "A1", "links": [[l]]})).unwrap();
        let got = s.execute("source.get", json!({"cell": "A1"})).unwrap();
        assert!(got["link"].get("hash").is_none() && got["link"].get("size").is_none());
    }

    #[test]
    fn list_returns_every_link_in_sheet_then_row_then_column_order() {
        let mut s = s();
        assert_eq!(s.execute("source.list", json!({})).unwrap(), json!({"links": [], "truncated": false}));
        s.execute("home.insertSheet", json!({"name": "Second"})).unwrap();
        // Set out of order, on both sheets.
        s.execute("source.set", json!({"sheet": "Second", "range": "A1", "links": [[link_json("s2a1")]]})).unwrap();
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "C2", "links": [[fingerprinted("c2")]]})).unwrap();
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "AA1", "links": [[link_json("aa1")]]})).unwrap();
        s.execute("source.set", json!({"sheet": "Sheet1", "range": "B2", "links": [[link_json("b2")]]})).unwrap();
        let order = s.doc().unwrap().wb.sheets.iter().map(|sh| sh.name.clone()).collect::<Vec<_>>();
        let r = s.execute("source.list", json!({})).unwrap();
        assert_eq!(r["truncated"], false);
        let got: Vec<(String, String, String)> = r["links"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["sheet"].as_str().unwrap().into(), x["cell"].as_str().unwrap().into(), x["link"]["locator"].as_str().unwrap().into()))
            .collect();
        let mut want = vec![
            ("Sheet1".to_string(), "AA1".to_string(), "aa1".to_string()),
            ("Sheet1".into(), "B2".into(), "b2".into()),
            ("Sheet1".into(), "C2".into(), "c2".into()),
        ];
        let second = ("Second".to_string(), "A1".to_string(), "s2a1".to_string());
        if order.first().map(String::as_str) == Some("Second") {
            want.insert(0, second);
        } else {
            want.push(second);
        }
        assert_eq!(got, want, "sheet order {order:?}");
        let c2 = r["links"].as_array().unwrap().iter().find(|x| x["cell"] == "C2").unwrap();
        assert_eq!(c2["link"], fingerprinted("c2"));
        // A query: no journal entry, no undo step.
        let undo = s.doc().unwrap().undo.len();
        assert!(s.journal.iter().all(|(id, _)| id != "source.list"));
        assert_eq!(s.doc().unwrap().undo.len(), undo);
    }

    #[test]
    fn set_missing_replaces_the_set_skips_bad_entries_and_is_not_an_edit() {
        let mut s = s();
        put(&mut s, "B2", 1.0, "b2");
        s.execute("home.insertSheet", json!({"name": "Other"})).unwrap();
        let dirty = s.doc().unwrap().is_dirty();
        let (undo, revision, journal) = (s.doc().unwrap().undo.len(), s.doc().unwrap().revision, s.journal.len());
        let r = s
            .execute(
                "source.setMissing",
                json!({"missing": [
                    {"sheet": "Sheet1", "cell": "B2"},
                    {"sheet": "other", "cell": "C3"},
                    {"sheet": "Nope", "cell": "A1"},
                    {"sheet": "Sheet1", "cell": "not a cell"},
                    {"sheet": "Sheet1", "cell": "XFE1"},
                    {"cell": "A1"},
                    5,
                    {"sheet": "Sheet1", "cell": "B2"}
                ]}),
            )
            .unwrap();
        assert_eq!(r, json!({"missing": 2, "skipped": 5}));
        let d = s.doc().unwrap();
        assert!(d.is_source_missing("Sheet1", CellRef::parse("B2").unwrap()));
        assert!(d.is_source_missing("Other", CellRef::parse("C3").unwrap()), "stored under the sheet's own name");
        assert_eq!((d.is_dirty(), d.undo.len(), d.revision, s.journal.len()), (dirty, undo, revision, journal));
        // Replaced, not added to; an empty list clears it.
        s.execute("source.setMissing", json!({"missing": [{"sheet": "Sheet1", "cell": "D4"}]})).unwrap();
        assert!(!s.doc().unwrap().is_source_missing("Sheet1", CellRef::parse("B2").unwrap()));
        s.execute("source.setMissing", json!({"missing": []})).unwrap();
        assert!(s.doc().unwrap().source_missing.is_empty());
        // Bad params change nothing.
        s.execute("source.setMissing", json!({"missing": [{"sheet": "Sheet1", "cell": "B2"}]})).unwrap();
        for p in [json!({}), json!({"missing": "B2"}), json!({"missing": {"sheet": "Sheet1"}})] {
            assert!(s.execute("source.setMissing", p).is_err());
        }
        assert!(s.doc().unwrap().is_source_missing("Sheet1", CellRef::parse("B2").unwrap()));
        // Never saved, and undo leaves it alone.
        let bytes = crate::io::save_bytes(&s.doc().unwrap().wb, "book.json").unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("issing"));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.doc().unwrap().is_source_missing("Sheet1", CellRef::parse("B2").unwrap()));
    }

    #[test]
    fn set_missing_refuses_an_oversized_list() {
        let mut s = s();
        let entries: Vec<Json> = (0..=super::MAX_MISSING).map(|_| json!(0)).collect();
        assert!(matches!(s.execute("source.setMissing", json!({"missing": entries})), Err(crate::EngineError::BadParams { .. })));
    }

    #[test]
    fn source_commands_survive_bad_params() {
        let mut s = s();
        for id in ["source.set", "source.get", "source.clear", "source.list", "source.setMissing"] {
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
                json!({"missing": [{"sheet": 3, "cell": 4}, {"sheet": "Sheet1", "cell": "ZZZ999999999"}, null]}),
                json!({"missing": null}),
                json!({"range": "A1", "links": [[{"file": "f", "locator": "l", "hash": "x", "size": -3}]]}),
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
}
