// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source markers: hovering one names the value's source.

use egui::{Event, Pos2};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::SourceLink;
use gridcraft_ui_egui::{Services, SheetApp, grid::Geo, source_marks, theme::Tokens};
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
fn tips_name_the_source_or_count_the_sources() {
    let s = session();
    let wb = &s.doc().unwrap().wb;
    let tip = |a: &str| source_marks::tip(wb, 0, CellRef::parse(a).unwrap());
    let statement = "Heritage Trust Bank Statement September 2019 · Beginning Balance";
    assert_eq!(tip("B2").as_deref(), Some(statement));
    assert_eq!(tip("B3").as_deref(), Some("No source for this value"));
    assert_eq!(tip("C2").as_deref(), Some(statement), "a formula with one source names it");
    assert_eq!(tip("C3").as_deref(), Some("1 input has no source"));
    assert_eq!(tip("C4").as_deref(), Some("From 2 sources"));
    assert_eq!(tip("C5").as_deref(), Some("Can't be traced to a source: it uses INDIRECT or OFFSET"));
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
    for cell in ["B2", "D2", "C2", "C4"] {
        assert!(blue.contains(&marker(&h, cell).min), "{cell} has a sourced marker: {blue:?}");
    }
    for cell in ["B3", "C3", "C5"] {
        assert!(amber.contains(&marker(&h, cell).min), "{cell} has an unsourced marker: {amber:?}");
    }
    assert_eq!((blue.len(), amber.len()), (4, 3), "only cells with a marker state get one");
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
/// of the running total. Run by hand: `cargo test --release -p gridcraft-ui-egui --test
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
