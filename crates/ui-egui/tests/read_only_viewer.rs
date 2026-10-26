// Added by Socratics, 2026: the read-only viewer. See MODIFICATIONS.md.
//! A read-only viewer looks like a workbook and refuses every edit, silently: typing, F2, a
//! double-click, Delete, paste, shortcuts, fill and resize drags, sheet tabs and API calls leave
//! the workbook clean at the same revision, with no message or dialog. Navigation, sheet
//! switching, zoom, copy and selection still work. The ribbon is the host's own switch.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::{EngineError, Session};
use gridcraft_ui_egui::{Chrome, SheetApp, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// Ctrl on Windows and Linux sets both `ctrl` and `command`.
const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

fn harness(read_only: bool, ribbon: bool) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2], [3, 4], [5, 6]]})).unwrap();
    s.execute("home.insertSheet", json!({"name": "Second"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    s.execute("selection.set", json!({"cell": "A1"})).unwrap();
    let d = s.active_mut().unwrap();
    d.saved = d.wb.clone();
    d.undo.clear();
    let mut app = SheetApp::new(s, Default::default());
    app.set_read_only(read_only);
    app.chrome = Chrome { ribbon, ..Chrome::default() };
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn viewer() -> Harness {
    harness(true, false)
}

/// The workbook's cells and sheets, as a value to compare.
fn content(h: &Harness) -> Vec<(String, Vec<(CellRef, Value)>)> {
    let d = h.state().session.active().unwrap();
    d.wb.sheets
        .iter()
        .map(|sh| {
            let mut cells: Vec<(CellRef, Value)> = (0..6).flat_map(|r| (0..4).map(move |c| CellRef::new(r, c))).map(|c| (c, sh.value(c))).collect();
            cells.retain(|(_, v)| *v != Value::Empty);
            (sh.name.clone(), cells)
        })
        .collect()
}

struct Snapshot {
    content: Vec<(String, Vec<(CellRef, Value)>)>,
    revision: u64,
    widths: Vec<f32>,
}

fn snapshot(h: &Harness) -> Snapshot {
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    Snapshot { content: content(h), revision: d.revision, widths: (0..4).map(|c| sh.col_width(c)).collect() }
}

/// Nothing changed, nothing is being edited, nothing was said.
fn assert_untouched(h: &Harness, before: &Snapshot, what: &str) {
    let app = h.state();
    let d = app.session.active().unwrap();
    assert_eq!(content(h), before.content, "{what}: cells and sheets unchanged");
    assert_eq!(d.revision, before.revision, "{what}: revision unchanged");
    assert!(!d.is_dirty(), "{what}: not dirty");
    let sh = d.wb.active().unwrap();
    assert_eq!((0..4).map(|c| sh.col_width(c)).collect::<Vec<_>>(), before.widths, "{what}: column widths unchanged");
    assert!(app.editor.is_none() && app.text_box_editor.is_none(), "{what}: no edit started");
    assert!(app.dialog.is_none(), "{what}: no dialog");
    assert!(app.message.is_none(), "{what}: no alert: {:?}", app.message);
    assert!(app.toast.is_none(), "{what}: no toast: {:?}", app.toast);
    assert!(app.grid.renaming_tab.is_none(), "{what}: no tab rename");
    assert!(app.session.clipboard.as_ref().is_none_or(|c| !c.cut), "{what}: no cut");
}

fn key(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn event(h: &mut Harness, e: Event) {
    h.input_mut().events.push(e);
    h.run_steps(2);
}

fn button(h: &mut Harness, pos: Pos2, b: PointerButton, pressed: bool) {
    h.input_mut().events.extend([Event::PointerMoved(pos), Event::PointerButton { pos, button: b, pressed, modifiers: Modifiers::NONE }]);
    h.step();
}

fn click(h: &mut Harness, pos: Pos2) {
    button(h, pos, PointerButton::Primary, true);
    button(h, pos, PointerButton::Primary, false);
    h.run_steps(2);
}

fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    button(h, from, PointerButton::Primary, true);
    for i in 1..=4 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.step();
    }
    button(h, to, PointerButton::Primary, false);
    h.run_steps(2);
}

fn geo(h: &Harness) -> (Geo, gridcraft_engine::model::Sheet) {
    let app = h.state();
    let sh = app.session.active().unwrap().wb.active().unwrap();
    (Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll), sh.clone())
}

fn cell_center(h: &Harness, a1: &str) -> Pos2 {
    let (geo, sh) = geo(h);
    geo.cell_rect(&sh, CellRef::parse(a1).unwrap()).center()
}

fn active(h: &Harness) -> CellRef {
    h.state().session.active().unwrap().selection.active
}

#[test]
fn typing_f2_double_click_and_enter_never_start_an_edit() {
    let mut h = viewer();
    let before = snapshot(&h);
    event(&mut h, Event::Text("9".into()));
    assert_untouched(&h, &before, "typing");
    key(&mut h, Key::F2, Modifiers::NONE);
    assert_untouched(&h, &before, "F2");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_untouched(&h, &before, "Enter");
    assert_eq!(active(&h), CellRef::new(1, 0), "Enter moves down");
    key(&mut h, Key::Enter, Modifiers { alt: true, ..Modifiers::NONE });
    assert_untouched(&h, &before, "Alt+Enter");
    let b2 = cell_center(&h, "B2");
    click(&mut h, b2);
    click(&mut h, b2);
    assert_untouched(&h, &before, "double-click");
    assert_eq!(active(&h), CellRef::new(1, 1), "the click selects");
    // The formula bar shows the cell but doesn't edit it.
    h.state_mut().begin_edit(None, true);
    h.run_steps(2);
    assert_untouched(&h, &before, "formula bar");
}

#[test]
fn delete_paste_and_shortcuts_are_refused_silently() {
    let mut h = viewer();
    h.state_mut().run("selection.set", json!({"range": "A1:B2"})).unwrap();
    let before = snapshot(&h);
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_untouched(&h, &before, "Delete");
    key(&mut h, Key::Backspace, Modifiers::NONE);
    assert_untouched(&h, &before, "Backspace");
    key(&mut h, Key::B, CTRL);
    assert_untouched(&h, &before, "Ctrl+B");
    key(&mut h, Key::D, CTRL);
    assert_untouched(&h, &before, "Ctrl+D");
    key(&mut h, Key::Num1, CTRL);
    assert_untouched(&h, &before, "Ctrl+1 (Format Cells)");
    key(&mut h, Key::H, CTRL);
    assert!(h.state().dialog.as_ref().is_some_and(|d| d.name() == "find"), "Ctrl+H opens Find");
    assert!(h.query_by_label("Replace All").is_none(), "without its Replace tab");
    h.state_mut().dialog = None;
    h.run_steps(2);
    event(&mut h, Event::Cut);
    assert_untouched(&h, &before, "Ctrl+X");
    event(&mut h, Event::Paste("7\t8".into()));
    assert_untouched(&h, &before, "Ctrl+V");
    event(&mut h, Event::Copy);
    assert!(h.state().session.clipboard.as_ref().is_some_and(|c| !c.cut), "Ctrl+C copies");
    key(&mut h, Key::Escape, Modifiers::NONE);
    h.state_mut().paste_from_clipboard();
    h.run_steps(2);
    assert_untouched(&h, &before, "Paste from the menu");
    key(&mut h, Key::Z, CTRL);
    assert_eq!(content(&h), before.content, "Ctrl+Z: nothing to undo");
}

#[test]
fn api_edits_are_refused_with_a_read_only_error() {
    let mut h = viewer();
    let before = snapshot(&h);
    for (id, params) in [
        ("cell.set", json!({"cell": "A1", "input": "x"})),
        ("range.setValues", json!({"range": "C1", "values": [[1]]})),
        ("source.set", json!({"range": "A1", "links": [[{"file": "a.pdf", "locator": "p1"}]]})),
        ("home.insertSheet", json!({})),
        ("sheet.rename", json!({"sheet": 0, "name": "X"})),
        ("home.deleteSheet", json!({"sheet": 1})),
        ("sheet.move", json!({"sheet": 0, "to": 1})),
        ("home.columnWidth", json!({"cols": "A:A", "width": 300})),
    ] {
        let r = h.state_mut().run(id, params.clone());
        let e = r.expect_err(id);
        assert!(EngineError::is_read_only_message(&e), "{id}: {e}");
        h.state_mut().run_or_alert(id, params);
        h.run_steps(2);
        assert_untouched(&h, &before, id);
    }
}

#[test]
fn fill_move_and_resize_drags_change_nothing() {
    let mut h = viewer();
    h.state_mut().run("selection.set", json!({"range": "A1:B2"})).unwrap();
    h.run_steps(2);
    let before = snapshot(&h);
    let (g, sh) = geo(&h);
    let corner = g.cell_rect(&sh, CellRef::new(1, 1)).right_bottom() - vec2(1.0, 1.0);
    drag(&mut h, corner, corner + vec2(0.0, 120.0));
    assert_untouched(&h, &before, "fill handle drag");
    // Double-click on the fill handle fills down elsewhere.
    h.state_mut().run("selection.set", json!({"range": "A1:B2"})).unwrap();
    h.run_steps(2);
    click(&mut h, corner);
    click(&mut h, corner);
    assert_untouched(&h, &before, "fill handle double-click");
    // Dragging the selection by its border moves it elsewhere.
    h.state_mut().run("selection.set", json!({"range": "A1:B2"})).unwrap();
    h.run_steps(2);
    let (g, sh) = geo(&h);
    let edge = g.cell_rect(&sh, CellRef::new(0, 1)).right_top() + vec2(0.0, 8.0);
    drag(&mut h, edge, edge + vec2(150.0, 60.0));
    assert_untouched(&h, &before, "move drag");
    // Column boundary drag and double-click.
    let (g, sh) = geo(&h);
    let boundary = egui::pos2(g.x(&sh, 1), g.rect.top() + g.header_h / 2.0);
    drag(&mut h, boundary, boundary + vec2(60.0, 0.0));
    assert_untouched(&h, &before, "column resize");
    click(&mut h, boundary);
    click(&mut h, boundary);
    assert_untouched(&h, &before, "column AutoFit");
    let row_boundary = egui::pos2(g.rect.left() + g.header_w / 2.0, g.y(&sh, 1));
    drag(&mut h, row_boundary, row_boundary + vec2(0.0, 40.0));
    assert_untouched(&h, &before, "row resize");
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().row_height(0), sh.row_height(0), "row height unchanged");
}

#[test]
fn context_menus_offer_copy_only() {
    let mut h = viewer();
    let b2 = cell_center(&h, "B2");
    button(&mut h, b2, PointerButton::Secondary, true);
    button(&mut h, b2, PointerButton::Secondary, false);
    h.run_steps(2);
    assert!(h.state().grid.context_menu.is_some(), "the menu opens");
    assert!(h.query_by_label("Copy").is_some(), "with Copy");
    for edit in ["Cut", "Paste", "Paste Special…", "Insert…", "Delete…", "Clear Contents", "Sort A to Z", "New Comment", "Format Cells…", "Link…"]
    {
        assert!(h.query_by_label(edit).is_none(), "no {edit}");
    }
    h.get_by_label("Copy").click();
    h.run_steps(2);
    assert!(h.state().session.clipboard.is_some(), "Copy copies");

    let (g, _) = geo(&h);
    let header = egui::pos2(g.rect.left() + g.header_w / 2.0, cell_center(&h, "A3").y);
    button(&mut h, header, PointerButton::Secondary, true);
    button(&mut h, header, PointerButton::Secondary, false);
    h.run_steps(2);
    assert!(h.state().grid.header_menu.is_some(), "the row header menu opens");
    assert!(h.query_by_label("Copy").is_some(), "with Copy");
    for edit in ["Insert", "Delete", "Row Height…", "Hide", "Unhide", "Group"] {
        assert!(h.query_by_label(edit).is_none(), "no {edit}");
    }
}

#[test]
fn sheet_tabs_switch_sheets_but_never_add_rename_or_move_one() {
    let mut h = viewer();
    assert!(h.query_by_label("New sheet").is_none(), "no New Sheet button");
    let editor = harness(false, true);
    assert!(editor.query_by_label("New sheet").is_some(), "an editor has one");
    let before = snapshot(&h);
    key(&mut h, Key::F11, Modifiers { shift: true, ..Modifiers::NONE });
    assert_untouched(&h, &before, "Shift+F11");
    key(&mut h, Key::PageDown, CTRL);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet, 1, "Ctrl+PageDown switches sheets");
    key(&mut h, Key::PageUp, CTRL);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet, 0);
    h.state_mut().run("sheet.activate", json!({"sheet": 1})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet, 1);
    h.state_mut().run("sheet.activate", json!({"sheet": 0})).unwrap();
    h.run_steps(2);
    // The active tab's name is painted text: find it in the tab bar, then double-click and drag it.
    let tab = h
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "Sheet1" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .expect("the Sheet1 tab is painted");
    click(&mut h, tab);
    click(&mut h, tab);
    h.run_steps(2);
    assert_untouched(&h, &before, "tab double-click");
    drag(&mut h, tab, tab + vec2(200.0, 0.0));
    assert_untouched(&h, &before, "tab drag");
    button(&mut h, tab, PointerButton::Secondary, true);
    button(&mut h, tab, PointerButton::Secondary, false);
    h.run_steps(2);
    assert!(h.query_by_label("Rename").is_none() && h.query_by_label("Insert Sheet").is_none(), "no tab menu");
    assert_untouched(&h, &before, "tab right-click");
}

#[test]
fn navigation_zoom_copy_and_selection_still_work() {
    let mut h = viewer();
    let before = snapshot(&h);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(active(&h), CellRef::new(1, 1));
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(active(&h), CellRef::new(1, 2));
    h.state_mut().run("selection.set", json!({"range": "A1:B3"})).unwrap();
    assert_eq!(h.state().session.active().unwrap().selection.a1(), "A1:B3");
    h.state_mut().run("view.zoom", json!({"percent": 150})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().zoom, 150, "zoom works");
    let copied = h.state_mut().run("edit.copy", json!({})).unwrap();
    assert!(copied["text"].as_str().is_some_and(|t| t.contains('3')), "copy gives the cells: {copied}");
    key(&mut h, Key::G, CTRL);
    assert!(h.state().dialog.as_ref().is_some_and(|d| d.name() == "goTo"), "Go To opens");
    h.state_mut().dialog = None;
    let d = h.state().session.active().unwrap();
    assert!(!d.is_dirty(), "view changes aren't edits");
    assert_eq!(d.revision, before.revision);
    assert_eq!(content(&h), before.content);
}

#[test]
fn the_host_can_hide_the_ribbon() {
    let full = harness(false, true);
    assert!(full.query_by_label("Paste").is_some(), "the ribbon is drawn by default");
    let mut editor = harness(false, false);
    assert!(editor.query_by_label("Paste").is_none(), "the host turned the ribbon off");
    let top = |h: &Harness| h.state().grid.rect.unwrap().top();
    assert!(top(&full) - top(&editor) > 60.0, "the grid moves up into the ribbon's place");
    let with_bar = top(&editor);
    editor.state_mut().run("view.formulaBar", json!({"on": false})).unwrap();
    editor.run_steps(3);
    assert!(with_bar - top(&editor) > 15.0, "the formula bar stays above it");
    let h = viewer();
    assert!(h.query_by_label("Paste").is_none(), "a viewer without a ribbon");
    assert!(h.state().grid.rect.is_some(), "still shows the grid");
}

#[test]
fn turning_read_only_on_drops_an_edit_in_progress_and_off_allows_edits_again() {
    let mut h = harness(false, true);
    h.state_mut().begin_edit(Some("99".into()), false);
    h.run_steps(2);
    assert!(h.state().editor.is_some());
    h.state_mut().set_read_only(true);
    h.run_steps(2);
    assert!(h.state().read_only());
    assert!(h.state().editor.is_none(), "the edit is dropped");
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0)), Value::Number(1.0), "not committed");
    h.state_mut().set_read_only(false);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
    assert!(h.state().session.active().unwrap().is_dirty(), "an editor edits again");
}
