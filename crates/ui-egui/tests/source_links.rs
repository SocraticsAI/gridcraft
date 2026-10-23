// Added by Socratics, 2026. See MODIFICATIONS.md.
//! The Source Inspector: markers on hard-coded values only (hovering one names the value's source;
//! a click only selects), and for an active linked value a tint and a source square whose click
//! tells the host. Formulas have none. View ▸ Source Inspector turns it all off.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::SourceLink;
use gridcraft_ui_egui::{Services, SheetApp, UiState, grid::Geo, source_marks, theme::Tokens};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const STATEMENT: &str = "Data Room/2nd Tier Requests/Cash/Heritage_Trust_Bank_Statement_September_2019.pdf";

/// B2 linked (1,318,609), B3 an unlinked 42, D2 linked to a ledger, and formulas over them.
fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "B2", "values": [[1318609], [42]]})).unwrap();
    s.execute("range.setValues", json!({"range": "D2", "values": [[10]]})).unwrap();
    s.execute(
        "source.set",
        json!({"range": "B2", "links": [[{"file": STATEMENT, "locator": "ACCOUNT SUMMARY / Beginning Balance", "storeRow": "Heritage__statement!C12"}]]}),
    )
    .unwrap();
    s.execute(
        "source.set",
        json!({"range": "D2", "links": [[{"file": "Data Room/GL/GL_Detail.xlsx", "locator": "GL Detail!C16102", "storeRow": "GL!C16102"}]]}),
    )
    .unwrap();
    for (cell, input) in [("C2", "=B2*2"), ("C3", "=B2+B3"), ("C4", "=B2+D2"), ("C5", "=INDIRECT(\"B2\")")] {
        s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
    }
    s
}

fn harness(session: Session, services: Services) -> Harness {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    h
}

fn marker(h: &Harness, cell: &str) -> egui::Rect {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    source_marks::marker_rect(&geo, sheet, CellRef::parse(cell).unwrap()).unwrap()
}

fn marker_center(h: &Harness, cell: &str) -> Pos2 {
    marker(h, cell).center()
}

fn tip_shown(h: &Harness) -> Option<String> {
    h.state().grid.source_tip.as_ref().and_then(|(_, t)| t.clone())
}

#[test]
fn tips_name_the_source_of_hard_coded_values_only() {
    let s = session();
    let wb = &s.doc().unwrap().wb;
    let tip = |a: &str| source_marks::tip(wb, 0, CellRef::parse(a).unwrap());
    assert_eq!(tip("B2").as_deref(), Some("Heritage Trust Bank Statement September 2019 · Beginning Balance"));
    assert_eq!(tip("B3").as_deref(), Some("No source for this value"));
    assert_eq!(tip("D2").as_deref(), Some("GL Detail · GL Detail!C16102"));
    for formula in ["C2", "C3", "C4", "C5"] {
        assert_eq!(tip(formula), None, "{formula} is a formula");
    }
    let ledger = SourceLink { file: "Data Room/GL/GL_Detail.xlsx".into(), locator: "GL Detail!C16102".into(), store_row: String::new() };
    assert_eq!(source_marks::label(&ledger), "GL Detail · GL Detail!C16102");
}

#[test]
fn hovering_a_marker_names_its_source() {
    let mut h = harness(session(), Services::default());
    let at = marker_center(&h, "B2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(tip_shown(&h).as_deref(), Some("Heritage Trust Bank Statement September 2019 · Beginning Balance"));
    let at = marker_center(&h, "B3");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(tip_shown(&h).as_deref(), Some("No source for this value"));
}

#[test]
fn a_formula_over_linked_values_has_no_marker_to_hover() {
    let mut h = harness(session(), Services::default());
    let at = marker_center(&h, "C2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None);
}

#[test]
fn a_workbook_without_links_has_no_marker_to_hover() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "B2", "values": [[1]]})).unwrap();
    let mut h = harness(s, Services::default());
    let at = marker_center(&h, "B2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None);
}

type Clicks = Rc<RefCell<Vec<(String, String)>>>;

fn with_clicks() -> (Harness, Clicks) {
    let clicks: Clicks = Rc::default();
    let seen = clicks.clone();
    let services = Services {
        source_click: Some(Box::new(move |sheet: &str, cell: &str| seen.borrow_mut().push((sheet.to_string(), cell.to_string())))),
        ..Default::default()
    };
    (harness(session(), services), clicks)
}

fn click(h: &mut Harness, pos: Pos2) {
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
}

fn active(h: &Harness) -> String {
    h.state().session.active().unwrap().selection.active.a1()
}

#[test]
fn clicking_a_marker_only_selects_the_cell() {
    let (mut h, clicks) = with_clicks();
    for cell in ["D2", "B3"] {
        let at = marker_center(&h, cell);
        click(&mut h, at);
        assert_eq!(active(&h), cell, "a click on the marker selects {cell}");
    }
    assert!(clicks.borrow().is_empty(), "markers are indicators: only the source square tells the host");
}

#[test]
fn clicking_elsewhere_in_the_cell_only_selects_it() {
    let (mut h, clicks) = with_clicks();
    let center = cell_rect(&h, "B2").center();
    click(&mut h, center);
    assert_eq!(active(&h), "B2");
    assert!(clicks.borrow().is_empty());
}

fn cell_rect(h: &Harness, cell: &str) -> egui::Rect {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll).cell_rect(sheet, CellRef::parse(cell).unwrap())
}

fn select(h: &mut Harness, cell: &str) {
    h.state_mut().session.execute("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
}

/// The source square shown this frame, in logical coordinates.
fn square(h: &Harness) -> Option<source_marks::SourceSquare> {
    h.state().grid.source_square
}

/// The painted rectangles filled with `color`.
fn rects(h: &Harness, color: egui::Color32) -> Vec<egui::Rect> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == color => Some(r.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn the_inspector_is_on_by_default_toggles_and_persists() {
    assert!(UiState::default().source_inspector);
    let old: UiState = serde_json::from_str(r#"{"dark": false, "animations": true}"#).unwrap();
    assert!(old.source_inspector, "a ui.json written before the setting existed keeps it on");
    let mut h = harness(session(), Services::default());
    assert!(h.state().ui.source_inspector);
    assert_eq!(h.state_mut().run("view.sourceInspector", json!({})), Ok(json!({"on": false})), "no params toggles");
    assert_eq!(h.state_mut().run("view.sourceInspector", json!({"on": false})), Ok(json!({"on": false})));
    let saved: UiState = serde_json::from_str(&serde_json::to_string(&h.state().ui).unwrap()).unwrap();
    assert!(!saved.source_inspector, "the choice is saved with the other UI preferences");
    assert_eq!(h.state_mut().run("view.sourceInspector", json!({"on": true})), Ok(json!({"on": true})));
    assert!(h.state().ui.source_inspector);
}

#[test]
fn the_ribbon_toggle_sits_at_the_end_of_the_view_tab() {
    let mut h = harness(session(), Services::default());
    h.state_mut().run("ui.ribbonTab", json!({"tab": "View"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Source Inspector").click();
    h.run_steps(2);
    assert!(!h.state().ui.source_inspector, "a click turns it off");
    assert!(h.state().session.journal.iter().all(|(id, _)| id != "view.sourceInspector"), "a UI preference, not a workbook edit");
}

#[test]
fn selecting_a_linked_cell_tints_it_and_shows_its_square_to_the_right() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    assert_eq!(square(&h), None, "A1 is empty");
    assert!(rects(&h, t.source_selected_fill).is_empty());
    select(&mut h, "B2");
    let b2 = cell_rect(&h, "B2");
    assert_eq!(rects(&h, t.source_selected_fill), [b2], "the active linked cell is tinted");
    let sq = square(&h).expect("a source square");
    assert_eq!(sq.cell.a1(), "B2");
    assert!(sq.rect.left() > b2.right() && sq.rect.left() < b2.right() + 6.0, "just right of the cell: {sq:?} {b2:?}");
    assert!((sq.rect.width() - sq.rect.height()).abs() < 0.01 && (sq.rect.height() - b2.height().max(source_marks::SQUARE_MIN)).abs() < 0.01);
    assert!((sq.rect.center().y - b2.center().y).abs() < 0.01, "level with the cell");
    assert_eq!(rects(&h, t.source_square_fill), [sq.rect], "painted in the chip colour");
}

#[test]
fn an_unsourced_or_formula_active_cell_has_no_tint_or_square() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    for cell in ["B3", "C2", "C4"] {
        select(&mut h, cell);
        assert_eq!(square(&h), None, "{cell}");
        assert!(rects(&h, t.source_selected_fill).is_empty() && rects(&h, t.source_square_fill).is_empty(), "{cell}");
    }
}

#[test]
fn only_the_active_cell_of_a_range_is_inspected() {
    let mut h = harness(session(), Services::default());
    h.state_mut().session.execute("selection.set", json!({"range": "B2:D3", "active": "B3"})).unwrap();
    h.run_steps(2);
    assert_eq!(square(&h), None, "B3 is unlinked, though B2 and D2 in the range are linked");
    h.state_mut().session.execute("selection.set", json!({"range": "B2:D3", "active": "D2"})).unwrap();
    h.run_steps(2);
    assert_eq!(square(&h).map(|s| s.cell.a1()).as_deref(), Some("D2"));
    assert_eq!(rects(&h, Tokens::light().source_selected_fill), [cell_rect(&h, "D2")]);
}

#[test]
fn clicking_the_square_tells_the_host_and_keeps_the_selection() {
    let (mut h, clicks) = with_clicks();
    select(&mut h, "D2");
    let at = square(&h).unwrap().rect.center();
    click(&mut h, at);
    assert_eq!(clicks.borrow().as_slice(), [("Sheet1".to_string(), "D2".to_string())]);
    assert_eq!(active(&h), "D2", "the square floats over E2 but the click doesn't select it");
}

#[test]
fn clicking_the_square_of_a_hyperlinked_cell_does_not_follow_the_link() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().session.execute("insert.link", json!({"cell": "B2", "target": "Sheet1!E9"})).unwrap();
    select(&mut h, "B2");
    let at = square(&h).unwrap().rect.center();
    click(&mut h, at);
    assert_eq!(clicks.borrow().len(), 1);
    assert_eq!(active(&h), "B2", "following the internal link would have gone to E9");
}

#[test]
fn hovering_the_square_names_the_source_and_lights_it() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    select(&mut h, "D2");
    let sq = square(&h).unwrap();
    h.input_mut().events.push(Event::PointerMoved(sq.rect.center()));
    h.run_steps(2);
    assert_eq!(tip_shown(&h).as_deref(), Some("GL Detail · GL Detail!C16102"));
    assert!(rects(&h, t.source_selected_fill).contains(&sq.rect), "the square deepens to the hover blue");
    assert!(rects(&h, t.source_square_fill).is_empty());
}

#[test]
fn the_fill_handle_wins_over_the_square() {
    let (mut h, clicks) = with_clicks();
    select(&mut h, "B2");
    let b2 = cell_rect(&h, "B2");
    let sq = square(&h).unwrap();
    // On both the square's bottom-left corner and B2's fill handle (9 points square on the corner).
    let at = egui::pos2(sq.rect.left() + 0.75, b2.bottom() - 1.0);
    assert!(sq.contains(at) && egui::Rect::from_center_size(b2.right_bottom(), egui::vec2(9.0, 9.0)).contains(at));
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None, "no source tooltip over the fill handle");
    assert!(rects(&h, Tokens::light().source_selected_fill).iter().all(|r| *r != sq.rect), "the square isn't lit");
    click(&mut h, at);
    assert!(clicks.borrow().is_empty(), "the press grabbed the fill handle");
    assert_eq!(active(&h), "B2");
}

#[test]
fn the_fill_handle_still_wins_over_a_diagonal_neighbours_marker() {
    let (mut h, clicks) = with_clicks();
    select(&mut h, "A1");
    // A1's fill handle overlaps B2's marker (B2 is sourced).
    let at = marker(&h, "B2").min + egui::vec2(3.0, 3.0);
    click(&mut h, at);
    assert_eq!(active(&h), "A1", "the click grabbed the handle instead of selecting B2");
    assert!(clicks.borrow().is_empty());
}

#[test]
fn turning_the_inspector_off_hides_markers_tint_square_and_tips() {
    let (mut h, clicks) = with_clicks();
    let t = Tokens::light();
    select(&mut h, "B2");
    let was = square(&h).unwrap().rect.center();
    h.state_mut().run("view.sourceInspector", json!({"on": false})).unwrap();
    h.run_steps(2);
    assert_eq!(square(&h), None);
    for color in [t.sourced_mark, t.unsourced_mark] {
        assert!(triangles(&h, color).is_empty(), "no markers");
    }
    assert!(rects(&h, t.source_selected_fill).is_empty() && rects(&h, t.source_square_fill).is_empty(), "no tint, no square");
    let at = marker_center(&h, "D2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None, "no tooltip on a hidden marker");
    click(&mut h, was);
    assert!(clicks.borrow().is_empty(), "nothing where the square was: the click selects the cell under it");
    assert_eq!(active(&h), "C2");
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(wb.active().unwrap().sources.len(), 2, "the links are untouched");
}

#[test]
fn the_square_flips_left_when_the_cell_runs_off_the_window() {
    let mut h = harness(session(), Services::default());
    h.state_mut().session.execute("home.columnWidth", json!({"cols": "B:B", "width": 1800})).unwrap();
    select(&mut h, "B2");
    let b2 = cell_rect(&h, "B2");
    let cells = h.state().grid.geo.as_ref().unwrap().cells;
    assert!(b2.right() > cells.right());
    let sq = square(&h).unwrap();
    assert!(sq.rect.right() < b2.left() && sq.rect.left() > cells.left(), "beside the cell's left edge, over A2: {sq:?}");
}

#[test]
fn the_square_sits_on_the_far_side_of_a_right_to_left_cell() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().session.execute("view.rightToLeft", json!({"on": true})).unwrap();
    select(&mut h, "B2");
    let geo = h.state().grid.geo.clone().unwrap();
    let sq = square(&h).unwrap();
    let b2 = geo.screen_cell_rect(h.state().session.active().unwrap().wb.active().unwrap(), CellRef::parse("B2").unwrap());
    let on_screen = geo.mirror_rect(sq.rect);
    assert!(on_screen.right() < b2.left(), "mirrored: left of the cell on screen ({on_screen:?}, {b2:?})");
    assert!(rects(&h, Tokens::light().source_square_fill).contains(&on_screen));
    click(&mut h, on_screen.center());
    assert_eq!(clicks.borrow().as_slice(), [("Sheet1".to_string(), "B2".to_string())]);
}

#[test]
fn the_square_of_a_frozen_cell_crosses_into_the_scrolled_columns_but_not_the_headers() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().run("view.freezePanes", json!({"cell": "C3"})).unwrap();
    select(&mut h, "B2");
    let sq = square(&h).unwrap();
    let geo = h.state().grid.geo.clone().unwrap();
    assert_eq!(sq.clip.top(), geo.cells.top(), "B2 is in the frozen rows");
    assert!(sq.clip.left() == geo.cells.left() && sq.visible() == sq.rect, "and in the frozen columns: the square is whole");
    click(&mut h, sq.rect.center());
    assert_eq!(clicks.borrow().len(), 1);
    // A scrolled cell under the frozen columns shows no square.
    select(&mut h, "D2");
    if let Some(v) = h.state_mut().view_mut() {
        v.scroll.x = 400.0;
    }
    h.run_steps(2);
    assert!(square(&h).is_none_or(|s| s.visible().left() >= geo.cells.left() + geo.frozen_w), "never over the frozen columns");
}

/// The top-left corners of the painted triangles of colour `color`.
fn triangles(h: &Harness, color: egui::Color32) -> Vec<Pos2> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(path) if path.fill == color && path.points.len() == 3 => path.points.first().copied(),
            _ => None,
        })
        .collect()
}

#[test]
fn markers_are_painted_in_the_top_left_corner_in_normal_view_only() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    let blue = triangles(&h, t.sourced_mark);
    let amber = triangles(&h, t.unsourced_mark);
    for cell in ["B2", "D2"] {
        assert!(blue.contains(&marker(&h, cell).min), "{cell} has a sourced marker: {blue:?}");
    }
    assert_eq!(amber, [marker(&h, "B3").min], "the unlinked number has an unsourced marker");
    assert_eq!(blue.len(), 2, "formulas (C2:C5) have no marker: {blue:?}");
    h.state_mut().run("view.pageLayout", json!({})).unwrap();
    h.run_steps(4);
    assert!(triangles(&h, t.sourced_mark).is_empty() && triangles(&h, t.unsourced_mark).is_empty(), "no markers in Page Layout view");
}

#[test]
fn page_layout_view_has_no_marker_to_hover() {
    let mut h = harness(session(), Services::default());
    h.state_mut().run("view.pageLayout", json!({})).unwrap();
    h.run_steps(4);
    let at = {
        let app = h.state();
        let sheet = app.session.active().unwrap().wb.active().unwrap();
        let geo = app.grid.geo.clone().unwrap();
        source_marks::marker_rect(&geo, sheet, CellRef::parse("B2").unwrap()).unwrap().center()
    };
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None);
}

/// 50,000 linked values (A1:E10000), a 5,000-formula running total over them (G1:G5000) and one
/// SUM over them all: how long the first frame after an edit takes, at the top and at the bottom
/// of the running total (whose formulas have no markers). Run by hand: `cargo test --release -p gridcraft-ui-egui --test
/// source_links -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run by hand"]
#[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
fn first_frame_after_an_edit_on_a_large_model() {
    let mut s = Session::new();
    s.new_workbook();
    let rows = 10_000;
    let values: Vec<Vec<f64>> = (0..rows).map(|r| (0..5).map(|c| (r * 5 + c) as f64).collect()).collect();
    s.execute("range.setValues", json!({"range": "A1", "values": values})).unwrap();
    let links: Vec<Vec<serde_json::Value>> = (0..rows)
        .map(|r| {
            (0..5).map(|c| json!({"file": "Data Room/GL/GL_Detail.xlsx", "locator": format!("GL Detail!C{}", r * 5 + c), "storeRow": ""})).collect()
        })
        .collect();
    s.execute("source.set", json!({"range": "A1:E10000", "links": links})).unwrap();
    s.execute("cell.set", json!({"cell": "G1", "input": "=SUM(A1:E1)"})).unwrap();
    s.execute("cell.set", json!({"cell": "G2", "input": "=G1+SUM(A2:E2)"})).unwrap();
    s.execute("selection.set", json!({"range": "G2:G5000"})).unwrap();
    s.execute("edit.fillDown", json!({})).unwrap();
    s.execute("cell.set", json!({"cell": "H1", "input": "=SUM(A1:E10000)"})).unwrap();
    s.execute("selection.set", json!({"cell": "J1"})).unwrap();
    let mut h = harness(s, Services::default());
    let frame = |h: &mut Harness| {
        let start = std::time::Instant::now();
        h.step();
        start.elapsed()
    };
    let idle = frame(&mut h);
    let edit = |h: &mut Harness, n: u32| {
        h.state_mut().run("cell.set", json!({"cell": "J1", "input": n.to_string()})).unwrap();
        frame(h)
    };
    let top = edit(&mut h, 1);
    let top_again = edit(&mut h, 2);
    let top_of_g4980 = h.state().session.active().unwrap().wb.active().unwrap().row_top(4979) as f32;
    if let Some(v) = h.state_mut().view_mut() {
        v.scroll.y = top_of_g4980;
    }
    h.step();
    let bottom = edit(&mut h, 3);
    let marks = triangles(&h, Tokens::light().sourced_mark).len();
    assert!(marks > 0 && h.state().view().scroll.y > 0.0);
    println!(
        "idle frame {idle:?}; first frame after an edit: top {top:?} (again {top_again:?}), bottom of the running total {bottom:?}; {marks} blue markers in view"
    );
}
