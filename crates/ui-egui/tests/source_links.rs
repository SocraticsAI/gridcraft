// Added by Socratics, 2026. See MODIFICATIONS.md.
//! The Source Inspector: markers on linked hard-coded numbers only (hovering one names the value's source),
//! a plain click on a linked value tells the host, and the active linked value is tinted. Formulas have
//! none. The Sources switch in the sheet-tab bar turns it all off.

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

const STATEMENT: &str = "Cash/Bank Statement September 2019.pdf";

/// B2 linked (1,318,609), B3 an unlinked 42, D2 linked to a ledger, and formulas over them.
fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "B2", "values": [[1318609], [42]]})).unwrap();
    s.execute("range.setValues", json!({"range": "D2", "values": [[10]]})).unwrap();
    s.execute("source.set", json!({"range": "B2", "links": [[{"file": STATEMENT, "locator": "page:3"}]]})).unwrap();
    s.execute("source.set", json!({"range": "D2", "links": [[{"file": "GL/GL Detail.xlsx", "locator": "GL Detail!C16102"}]]})).unwrap();
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
    assert_eq!(tip("B2").as_deref(), Some("Bank Statement September 2019 · Page 3"));
    assert_eq!(tip("B3"), None, "an unlinked number has no marker, so no tooltip");
    assert_eq!(tip("D2").as_deref(), Some("GL Detail · GL Detail!C16102"));
    for formula in ["C2", "C3", "C4", "C5"] {
        assert_eq!(tip(formula), None, "{formula} is a formula");
    }
    let label = |file: &str, locator: &str| source_marks::label(&SourceLink { file: file.into(), locator: locator.into() });
    assert_eq!(label("GL/GL Detail.xlsx", "GL Detail!C16102"), "GL Detail · GL Detail!C16102");
    assert_eq!(label("Deck.pptx", "slide:4"), "Deck · Slide 4");
    assert_eq!(label("notes.txt", "line:12"), "notes · Line 12");
    assert_eq!(label("Engagement Letter.docx", "document"), "Engagement Letter", "the whole file adds no place");
}

#[test]
fn hovering_a_marker_names_its_source() {
    let mut h = harness(session(), Services::default());
    let at = marker_center(&h, "B2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(tip_shown(&h).as_deref(), Some("Bank Statement September 2019 · Page 3"));
    let at = marker_center(&h, "B3");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(tip_shown(&h), None, "nothing to hover on an unlinked number");
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
        source_click: Some(Box::new(move |sheet: &str, cell: &str, _rect: Option<[f32; 4]>| {
            seen.borrow_mut().push((sheet.to_string(), cell.to_string()))
        })),
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
fn clicking_a_linked_cell_selects_it_and_tells_the_host() {
    let (mut h, clicks) = with_clicks();
    let on_marker = marker_center(&h, "D2");
    click(&mut h, on_marker);
    assert_eq!(active(&h), "D2", "a click on the marker selects the cell");
    let center = cell_rect(&h, "B2").center();
    click(&mut h, center);
    assert_eq!(active(&h), "B2");
    assert_eq!(clicks.borrow().as_slice(), [("Sheet1".to_string(), "D2".to_string()), ("Sheet1".to_string(), "B2".to_string())]);
}

#[test]
fn a_click_on_a_linked_cell_says_where_the_cell_is() {
    let rects: Rc<RefCell<Vec<Option<[f32; 4]>>>> = Rc::default();
    let seen = rects.clone();
    let services =
        Services { source_click: Some(Box::new(move |_: &str, _: &str, rect: Option<[f32; 4]>| seen.borrow_mut().push(rect))), ..Default::default() };
    let mut h = harness(session(), services);
    let d2 = cell_rect(&h, "D2");
    click(&mut h, d2.center());
    let got = rects.borrow()[0].expect("a rectangle");
    let want = [d2.left(), d2.top(), d2.width(), d2.height()];
    assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01), "the cell on screen, in points at UI zoom 1: {got:?} {want:?}");
}

#[test]
fn a_modifier_click_on_a_linked_cell_only_selects() {
    let (mut h, clicks) = with_clicks();
    let at = cell_rect(&h, "B2").center();
    for modifiers in [Modifiers::SHIFT, Modifiers::COMMAND] {
        h.input_mut().events.push(Event::ModifiersChanged(modifiers));
        h.input_mut().events.extend([
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers },
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers },
        ]);
        h.run_steps(2);
    }
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    assert!(clicks.borrow().is_empty(), "extending or adding to a selection is not asking for the source");
}

#[test]
fn double_clicking_a_linked_cell_still_edits_it() {
    let (mut h, clicks) = with_clicks();
    let at = cell_rect(&h, "B2").center();
    click(&mut h, at);
    click(&mut h, at);
    assert!(h.state().editor.is_some(), "the second click of a double-click edits");
    assert_eq!(clicks.borrow().len(), 1, "only the first click asked for the source");
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

/// The unoutlined rectangles filled with `color` painted over the grid (the selection tint).
fn rects(h: &Harness, color: egui::Color32) -> Vec<egui::Rect> {
    let grid = h.state().grid.rect.unwrap();
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == color && grid.intersects(r.rect) && r.stroke.width == 0.0 => Some(r.rect),
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

/// Where the sheet-tab bar paints tab label `label` (below the grid).
fn sheet_tab_label(h: &Harness, label: &str) -> egui::Rect {
    let grid_bottom = h.state().grid.rect.unwrap().bottom();
    h.output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label && t.pos.y > grid_bottom => Some(t.visual_bounding_rect()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label} sheet tab"))
}

#[test]
fn the_sources_switch_sits_in_the_sheet_tab_bar_after_new_sheet() {
    let h = harness(session(), Services::default());
    let switch = h.get_by_label("Sources").rect();
    let grid = h.state().grid.rect.unwrap();
    assert!(switch.top() > grid.bottom(), "below the grid, in the sheet-tab bar: {switch:?} {grid:?}");
    let new_sheet = h.get_by_label("New sheet").rect();
    assert!(switch.left() > new_sheet.right(), "after New Sheet: {switch:?} {new_sheet:?}");
    assert!((switch.center().y - new_sheet.center().y).abs() < 3.0, "level with the tabs: {switch:?} {new_sheet:?}");
    assert!(switch.right() < grid.left() + grid.width() * 0.8, "the scrollbar keeps the rest of the bar: {switch:?}");
}

#[test]
fn new_sheet_follows_the_last_tab() {
    let h = harness(session(), Services::default());
    let new_sheet = h.get_by_label("New sheet").rect();
    let last = sheet_tab_label(&h, "Sheet1");
    assert!(new_sheet.left() > last.right() && new_sheet.left() < last.right() + 40.0, "right after the last tab: {new_sheet:?} {last:?}");
}

#[test]
fn the_sources_switch_flips_the_inspector_and_edits_nothing() {
    let mut h = harness(session(), Services::default());
    h.get_by_label("Sources").click();
    h.run_steps(2);
    assert!(!h.state().ui.source_inspector, "a click turns it off");
    assert!(h.state().session.journal.iter().all(|(id, _)| id != "view.sourceInspector"), "a UI preference, not a workbook edit");
    assert_eq!(h.state().session.doc().unwrap().wb.sheets.len(), 1, "no sheet added");
    h.get_by_label("Sources").click();
    h.run_steps(2);
    assert!(h.state().ui.source_inspector, "a second click turns it back on");
}

/// The shapes painted inside `rect`.
fn shapes_in(h: &Harness, rect: egui::Rect) -> Vec<egui::Shape> {
    h.output().shapes.iter().filter(|c| rect.contains_rect(c.shape.visual_bounding_rect())).map(|c| c.shape.clone()).collect()
}

/// The small switch's pill track among `shapes`: a filled rect 16 px high.
fn track(shapes: &[egui::Shape]) -> egui::epaint::RectShape {
    shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT && (r.rect.height() - 16.0).abs() < 0.5 => Some(r.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no track: {shapes:?}"))
}

/// The thumb's centre among `shapes`: the switch-thumb-coloured circle.
fn thumb(shapes: &[egui::Shape], t: &Tokens) -> egui::Pos2 {
    shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Circle(c) if c.fill == t.switch_thumb => Some(c.center),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no thumb: {shapes:?}"))
}

#[test]
fn the_sources_switch_shows_its_state() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    let rect = h.get_by_label("Sources").rect();
    // Away from the switch, so it is drawn at rest.
    h.input_mut().events.push(Event::PointerMoved(Pos2::new(5.0, 5.0)));
    h.run_steps(2);
    let on = shapes_in(&h, rect);
    let pill = track(&on);
    assert_eq!(pill.rect.size(), egui::vec2(32.0, 16.0));
    assert_eq!(pill.fill, t.switch_on);
    assert!(thumb(&on, &t).x > pill.rect.center().x, "the thumb sits right while on");
    let label = on.iter().find_map(|s| match s {
        egui::Shape::Text(x) if x.galley.text() == "Sources" => Some(x.visual_bounding_rect()),
        _ => None,
    });
    assert!(label.expect("the label").right() < pill.rect.left(), "the label comes before the switch");
    let label_color = on.iter().find_map(|s| match s {
        egui::Shape::Text(x) if x.galley.text() == "Sources" => Some(x.galley.job.sections[0].format.color),
        _ => None,
    });
    assert_eq!(label_color, Some(t.text), "the sheet tabs' text colour");
    h.get_by_label("Sources").click();
    h.run_steps(2);
    h.input_mut().events.push(Event::PointerMoved(Pos2::new(5.0, 5.0)));
    h.run_steps(2);
    let off = shapes_in(&h, rect);
    assert_eq!(track(&off).fill, t.switch_off);
    assert!(thumb(&off, &t).x < pill.rect.center().x, "the thumb sits left while off");
}

#[test]
fn selecting_a_linked_cell_tints_it() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    assert!(rects(&h, t.source_selected_fill).is_empty(), "A1 is empty");
    select(&mut h, "B2");
    let b2 = cell_rect(&h, "B2");
    assert_eq!(rects(&h, t.source_selected_fill), [b2], "the active linked cell is tinted");
}

#[test]
fn an_unlinked_number_shows_nothing_even_beside_links() {
    let (mut h, clicks) = with_clicks();
    let t = Tokens::light();
    let b3 = marker(&h, "B3").min;
    assert!(!marks(&h, t.sourced_mark).contains(&b3), "no marker on B3");
    select(&mut h, "B3");
    assert!(rects(&h, t.source_selected_fill).is_empty(), "no tint");
    let at = cell_rect(&h, "B3").center();
    click(&mut h, at);
    assert!(clicks.borrow().is_empty(), "nothing to open");
    assert_eq!(active(&h), "B3");
}

#[test]
fn a_formula_or_unlinked_text_cell_has_no_tint_and_its_click_opens_nothing() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().session.execute("cell.set", json!({"cell": "E3", "input": "Opening balance"})).unwrap();
    let t = Tokens::light();
    for cell in ["C2", "C4", "E3"] {
        let at = cell_rect(&h, cell).center();
        click(&mut h, at);
        assert_eq!(active(&h), cell);
        assert!(rects(&h, t.source_selected_fill).is_empty(), "{cell}");
    }
    assert!(clicks.borrow().is_empty());
}

#[test]
fn only_the_active_cell_of_a_range_is_inspected() {
    let mut h = harness(session(), Services::default());
    h.state_mut().session.execute("selection.set", json!({"range": "B2:D3", "active": "B3"})).unwrap();
    h.run_steps(2);
    assert!(rects(&h, Tokens::light().source_selected_fill).is_empty(), "B3 is unlinked, though B2 and D2 in the range are linked");
    h.state_mut().session.execute("selection.set", json!({"range": "B2:D3", "active": "D2"})).unwrap();
    h.run_steps(2);
    assert_eq!(rects(&h, Tokens::light().source_selected_fill), [cell_rect(&h, "D2")]);
}

#[test]
fn a_linked_cell_with_a_hyperlink_follows_the_link_instead() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().session.execute("insert.link", json!({"cell": "B2", "target": "Sheet1!E9"})).unwrap();
    let b2 = cell_rect(&h, "B2");
    click(&mut h, egui::pos2(b2.left() + 6.0, b2.center().y));
    assert_eq!(active(&h), "E9", "the link was followed");
    assert!(clicks.borrow().is_empty(), "and the source was not asked for");
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
fn turning_the_inspector_off_hides_markers_tint_and_tips_and_clicks_open_nothing() {
    let (mut h, clicks) = with_clicks();
    let t = Tokens::light();
    select(&mut h, "B2");
    h.state_mut().run("view.sourceInspector", json!({"on": false})).unwrap();
    h.run_steps(2);
    assert!(marks(&h, t.sourced_mark).is_empty(), "no markers");
    assert!(rects(&h, t.source_selected_fill).is_empty(), "no tint");
    let at = marker_center(&h, "D2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().grid.source_tip, None, "no tooltip on a hidden marker");
    let at = cell_rect(&h, "D2").center();
    click(&mut h, at);
    assert!(clicks.borrow().is_empty(), "a click on a linked cell only selects it");
    assert_eq!(active(&h), "D2");
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(wb.active().unwrap().sources.len(), 2, "the links are untouched");
}

#[test]
fn clicking_a_linked_cell_of_a_right_to_left_sheet_tells_the_host() {
    let (mut h, clicks) = with_clicks();
    h.state_mut().session.execute("view.rightToLeft", json!({"on": true})).unwrap();
    h.run_steps(2);
    let geo = h.state().grid.geo.clone().unwrap();
    let b2 = geo.screen_cell_rect(h.state().session.active().unwrap().wb.active().unwrap(), CellRef::parse("B2").unwrap());
    click(&mut h, b2.center());
    assert_eq!(clicks.borrow().as_slice(), [("Sheet1".to_string(), "B2".to_string())]);
}

/// The top-left corners of the painted marker squares of colour `color`.
fn marks(h: &Harness, color: egui::Color32) -> Vec<Pos2> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == color && (r.rect.width() - r.rect.height()).abs() < 0.01 && r.rect.width() < 8.0 => Some(r.rect.min),
            _ => None,
        })
        .collect()
}

#[test]
fn markers_are_painted_in_the_top_left_corner_in_normal_view_only() {
    let mut h = harness(session(), Services::default());
    let t = Tokens::light();
    let blue = marks(&h, t.sourced_mark);
    for cell in ["B2", "D2"] {
        assert!(blue.contains(&marker(&h, cell).min), "{cell} has a sourced marker: {blue:?}");
    }
    assert_eq!(blue.len(), 2, "the unlinked number B3 and the formulas (C2:C5) have no marker: {blue:?}");
    h.state_mut().run("view.pageLayout", json!({})).unwrap();
    h.run_steps(4);
    assert!(marks(&h, t.sourced_mark).is_empty(), "no markers in Page Layout view");
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

#[test]
fn a_linked_label_beside_a_linked_figure_shows_nothing() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A2", "input": "Beginning Balance"})).unwrap();
    s.execute("source.set", json!({"range": "A2", "links": [[{"file": STATEMENT, "locator": "page:1"}]]})).unwrap();
    let mut h = harness(s, Services::default());
    let t = Tokens::light();
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(source_marks::tip(wb, 0, CellRef::parse("A2").unwrap()), None, "no tooltip for a label");
    let blue = marks(&h, t.sourced_mark);
    assert!(blue.contains(&marker(&h, "B2").min), "the figure beside it is blue");
    assert!(!blue.contains(&marker(&h, "A2").min), "the label has no marker");
    let at = marker_center(&h, "A2");
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(tip_shown(&h), None);
    select(&mut h, "A2");
    assert!(rects(&h, t.source_selected_fill).is_empty(), "no tint for a label");
    assert!(h.state().session.active().unwrap().wb.active().unwrap().sources.contains_key(&CellRef::parse("A2").unwrap()), "the link stays");
}
