// Added by Socratics, 2026: read-only sessions. See MODIFICATIONS.md.
//! A read-only session (a viewer) refuses every command that would change a workbook and still
//! runs those that only look at it or change its view.

use std::sync::Arc;

use gridcraft_engine::core::CellRef;
use gridcraft_engine::{EngineError, Session, command_specs};
use serde_json::{Value as Json, json};

fn viewer() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"sample": "sales"})).unwrap();
    s.execute("home.insertSheet", json!({"name": "Notes"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let d = s.active_mut().unwrap();
    d.saved = d.wb.clone();
    d.undo.clear();
    d.redo.clear();
    s.journal.clear();
    s.read_only = true;
    s
}

/// What a refused command must leave as it was.
struct Before {
    wb: Arc<gridcraft_engine::model::Workbook>,
    revision: u64,
    selection: gridcraft_engine::Selection,
    journal: usize,
}

fn before(s: &Session) -> Before {
    let d = s.doc().unwrap();
    Before { wb: d.wb.clone(), revision: d.revision, selection: d.selection.clone(), journal: s.journal.len() }
}

fn assert_refused(s: &mut Session, id: &str, params: Json) {
    let b = before(s);
    let r = s.execute(id, params.clone());
    assert!(matches!(r, Err(EngineError::ReadOnly(_))), "{id} {params}: {r:?}");
    let message = r.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(EngineError::is_read_only_message(&message), "{message}");
    let d = s.doc().unwrap();
    assert!(Arc::ptr_eq(&d.wb, &b.wb), "{id} left the workbook as it was");
    assert!(!d.is_dirty(), "{id}: not dirty");
    assert_eq!(d.revision, b.revision, "{id}: revision");
    assert_eq!(d.selection, b.selection, "{id}: selection");
    assert!(d.undo.is_empty() && d.redo.is_empty(), "{id}: no history");
    assert_eq!(s.journal.len(), b.journal, "{id}: not journaled");
    assert!(s.take_ui_requests().is_empty(), "{id}: no dialog or edit mode");
    assert!(s.format_painter.is_none() && s.draw_tool.is_empty(), "{id}: no tool armed");
    assert!(s.clipboard.as_ref().is_none_or(|c| !c.cut), "{id}: no cut");
}

#[test]
fn every_edit_is_refused_and_leaves_the_workbook_as_it_was() {
    let mut s = viewer();
    s.execute("selection.set", json!({"range": "B2:C3"})).unwrap();
    for (id, params) in [
        ("cell.set", json!({"cell": "B2", "input": "42"})),
        ("cell.set", json!({"cell": "Z99", "input": "=1+1"})),
        ("range.setValues", json!({"range": "A1", "values": [[1, 2], [3, 4]]})),
        ("source.set", json!({"range": "B2", "links": [[{"file": "a.pdf", "locator": "p1"}]]})),
        ("source.clear", json!({"range": "A1:Z99"})),
        ("home.bold", json!({})),
        ("home.numberFormat", json!({"code": "0%"})),
        ("edit.clearContents", json!({})),
        ("edit.cut", json!({})),
        ("edit.fillDown", json!({})),
        ("edit.autoFill", json!({"source": "B2", "target": "B2:B9"})),
        ("edit.formatPainter", json!({})),
        ("draw.pen", json!({})),
        ("home.columnWidth", json!({"cols": "B:B", "width": 200})),
        ("home.rowHeight", json!({"rows": "2:2", "height": 60})),
        ("home.insertRows", json!({})),
        ("home.insertSheet", json!({})),
        ("home.deleteSheet", json!({"sheet": 1})),
        ("sheet.rename", json!({"sheet": 0, "name": "Renamed"})),
        ("sheet.move", json!({"sheet": 0, "to": 1})),
        ("sheet.tabColor", json!({"sheet": 0, "color": "#FF0000"})),
        ("view.freezePanes", json!({"cell": "B2"})),
        ("view.gridlines", json!({"on": false})),
        ("data.sortAscending", json!({})),
        ("data.filter", json!({})),
        ("insert.table", json!({"range": "K20:L25"})),
        ("formulas.defineName", json!({"name": "Total", "refersTo": "=Sheet1!$B$2"})),
    ] {
        assert_refused(&mut s, id, params);
    }
}

#[test]
fn paste_and_undo_are_refused() {
    let mut s = viewer();
    s.execute("selection.set", json!({"range": "A1:B2"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.clipboard.as_ref().is_some_and(|c| !c.cut), "copy works");
    assert_refused(&mut s, "edit.paste", json!({"at": "F10"}));
    assert_refused(&mut s, "edit.paste", json!({"at": "F10", "text": "1\t2"}));
    assert_refused(&mut s, "edit.pasteSpecial", json!({"what": "values"}));
    // Nothing to undo: a viewer keeps no history.
    assert!(matches!(s.execute("edit.undo", json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn the_workbook_file_is_never_written() {
    let mut s = viewer();
    let dir = std::env::temp_dir().join(format!("gridcraft-read-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("viewer.xlsx");
    let path = path.to_str().unwrap();
    assert_refused(&mut s, "file.save", json!({"path": path}));
    assert_refused(&mut s, "file.saveAs", json!({"path": path}));
    assert!(!std::path::Path::new(path).exists(), "no file written");
    // Getting the bytes (to export a copy) isn't a write of the workbook's file.
    assert!(s.execute("file.saveBytes", json!({"format": "xlsx"})).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn navigation_and_view_still_work_without_dirtying_the_workbook() {
    let mut s = viewer();
    let revision = s.doc().unwrap().revision;
    s.execute("selection.set", json!({"range": "C3:D5"})).unwrap();
    assert_eq!(s.doc().unwrap().selection.a1(), "C3:D5");
    s.execute("selection.move", json!({"dr": 1, "dc": 0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.active, CellRef::new(3, 2));
    s.execute("edit.selectAll", json!({})).unwrap();
    s.execute("edit.goTo", json!({"reference": "B4"})).unwrap();
    assert_eq!(s.doc().unwrap().selection.active, CellRef::new(3, 1));
    s.execute("sheet.next", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active_sheet, 1);
    s.execute("sheet.previous", json!({})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 1})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active_sheet, 1);
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    s.execute("view.zoom", json!({"percent": 150})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active().unwrap().zoom, 150);
    s.execute("view.pageLayout", json!({})).unwrap();
    s.execute("view.normal", json!({})).unwrap();
    let found = s.execute("edit.find", json!({"what": "a", "all": true})).unwrap();
    assert!(found.get("results").is_some(), "find works: {found}");
    s.execute("selection.set", json!({"range": "A1:B2"})).unwrap();
    let copied = s.execute("edit.copy", json!({})).unwrap();
    assert!(copied.get("text").and_then(Json::as_str).is_some_and(|t| !t.is_empty()), "copy gives the cells' text");
    assert!(s.execute("cell.get", json!({"cell": "A1"})).is_ok());
    assert!(s.execute("sheet.read", json!({})).is_ok());
    assert!(s.execute("source.get", json!({"cell": "B2"})).is_ok());
    let d = s.doc().unwrap();
    assert!(!d.is_dirty(), "view changes are not edits");
    assert_eq!(d.revision, revision, "a viewer's revision doesn't move");
    assert!(d.undo.is_empty(), "no history");
}

#[test]
fn a_workbook_with_unsaved_changes_stays_dirty_and_still_refuses_edits() {
    let mut s = viewer();
    s.read_only = false;
    s.execute("cell.set", json!({"cell": "A1", "input": "changed"})).unwrap();
    s.read_only = true;
    let wb = s.doc().unwrap().wb.clone();
    assert!(s.doc().unwrap().is_dirty());
    s.execute("view.zoom", json!({"percent": 80})).unwrap();
    s.execute("sheet.next", json!({})).unwrap();
    assert!(s.doc().unwrap().is_dirty(), "still dirty after view changes");
    let r = s.execute("cell.set", json!({"cell": "A2", "input": "x"}));
    assert!(matches!(r, Err(EngineError::ReadOnly(_))), "{r:?}");
    s.execute("sheet.previous", json!({})).unwrap();
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.value(CellRef::new(1, 0)), wb.active().unwrap().value(CellRef::new(1, 0)), "A2 unchanged");
    assert!(s.doc().unwrap().is_dirty(), "the earlier change is still unsaved");
}

#[test]
fn turning_read_only_off_makes_the_workbook_editable_again() {
    let mut s = viewer();
    assert!(s.execute("cell.set", json!({"cell": "A1", "input": "1"})).is_err());
    s.read_only = false;
    s.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    assert!(s.doc().unwrap().is_dirty());
}

/// Every command, with no params and with hostile ones, neither panics nor changes a read-only
/// workbook.
#[test]
fn every_command_with_any_params_leaves_a_read_only_workbook_as_it_was() {
    let mut s = viewer();
    let start = s.doc().unwrap().wb.clone();
    for spec in command_specs() {
        // These open or close documents, or read files.
        if matches!(
            spec.id,
            "file.close" | "file.open" | "file.new" | "window.activate" | "view.newWindow" | "insert.picture" | "pageLayout.background"
        ) {
            continue;
        }
        for params in [json!({}), json!([1, 2]), json!({"range": "ZZZ999999999", "cell": "", "sheet": 99}), json!({"range": "A1:C3", "cell": "B2"})] {
            let r = s.execute(spec.id, params.clone());
            assert!(!matches!(r, Err(EngineError::Internal(..))), "{} {params}: {r:?}", spec.id);
            s.take_ui_requests();
            assert_eq!(s.documents().len(), 1, "{}", spec.id);
            let d = s.doc().unwrap();
            assert!(!d.is_dirty(), "{} {params} dirtied a read-only workbook", spec.id);
            assert!(d.undo.is_empty(), "{} {params} recorded history", spec.id);
            let content_kept = d.wb.sheets.len() == start.sheets.len()
                && d.wb.sheets.iter().zip(&start.sheets).all(|(a, b)| a.name == b.name && a.cells.diff(&b.cells).is_empty());
            assert!(content_kept, "{} {params} changed the cells", spec.id);
        }
    }
}
