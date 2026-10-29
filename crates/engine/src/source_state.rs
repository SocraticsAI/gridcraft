// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Which cells show as sourced: hard-coded numbers that carry a source link (the document they
//! were read from), read from the cell itself. [`is_sourced`] answers for one cell and
//! [`sourced_in`] for a range (what the grid paints). Linked text and logicals (labels, headers),
//! unlinked numbers, formulas and spilled values show nothing.

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::{Sheet, Workbook};

/// Whether `r` runs backwards (normalised ranges never do; the map's range lookups would panic).
fn is_inverted(r: RangeRef) -> bool {
    r.start.row > r.end.row || r.start.col > r.end.col
}

/// Whether `c` on `sh` shows as sourced, read from the cell alone: a hard-coded number with a
/// source link. Linked text and logicals (labels, headers), unlinked numbers, formulas and spilled
/// values (which have no stored value of their own) don't.
fn sourced_on(sh: &Sheet, c: CellRef) -> bool {
    sh.sources.contains_key(&c) && sh.cell(c).is_some_and(|x| x.formula.is_none() && x.value.is_number())
}

/// Whether one cell shows as sourced (carries a marker).
pub fn is_sourced(wb: &Workbook, sheet_index: usize, cell: CellRef) -> bool {
    wb.sheet(sheet_index).is_some_and(|sh| sourced_on(sh, cell))
}

/// The cells of `range` that show as sourced, row by row (what the grid paints for a visible
/// pane). Only linked cells are looked at.
pub fn sourced_in(wb: &Workbook, sheet_index: usize, range: RangeRef) -> Vec<CellRef> {
    let Some(sh) = wb.sheet(sheet_index) else { return Vec::new() };
    if is_inverted(range) {
        return Vec::new();
    }
    let cols = range.start.col..=range.end.col;
    sh.sources.range(range.start..=range.end).map(|(c, _)| *c).filter(|c| cols.contains(&c.col) && sourced_on(sh, *c)).collect()
}

#[cfg(test)]
mod tests {
    use gridcraft_core::{CellRef, RangeRef};
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::Session;

    fn s() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn link(locator: &str) -> Json {
        json!({"file": "Cash/Statement.pdf", "locator": locator})
    }

    fn state(s: &Session, cell: &str) -> bool {
        let wb = &s.doc().unwrap().wb;
        is_sourced(wb, wb.active_sheet, CellRef::parse(cell).unwrap())
    }

    fn shown(s: &Session, range: &str) -> Vec<String> {
        sourced_in(&s.doc().unwrap().wb, 0, RangeRef::parse(range).unwrap()).iter().map(|c| c.a1()).collect()
    }

    #[test]
    fn a_workbook_without_links_has_no_markers() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=A1+B1"})).unwrap();
        for c in ["A1", "B1", "C1"] {
            assert!(!state(&s, c), "{c}");
        }
        assert!(shown(&s, "A1:C1").is_empty());
    }

    #[test]
    fn only_hard_coded_values_have_markers() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100, 7, "text", true, "cash"]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("opening")]]})).unwrap();
        s.execute("source.set", json!({"range": "C1:E1", "links": [[link("label"), link("flag"), link("cash")]]})).unwrap();
        for (cell, input) in [("F1", "=A1*2"), ("G1", "=A1+B1"), ("H1", "=UPPER(C1)"), ("I1", "=INDIRECT(\"A1\")"), ("J1", "=F1"), ("K1", "=1+2")] {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        assert!(state(&s, "A1"), "a linked number");
        assert!(!state(&s, "B1"), "an unlinked number shows nothing, even beside links");
        assert!(!state(&s, "C1"), "linked text: a label, not a figure");
        assert!(!state(&s, "D1"), "a linked logical");
        assert!(!state(&s, "E1"), "linked text");
        for c in ["F1", "G1", "H1", "I1", "J1", "K1", "Z9"] {
            assert!(!state(&s, c), "{c}");
        }
        assert_eq!(shown(&s, "A1:K1"), ["A1"]);
        s.execute("source.clear", json!({"range": "C1:E1"})).unwrap();
        for c in ["C1", "D1", "E1"] {
            assert!(!state(&s, c), "unlinked {c}");
        }
        let wb = &s.doc().unwrap().wb;
        assert!(wb.active().unwrap().sources.contains_key(&CellRef::parse("A1").unwrap()), "only the text links went");
    }

    #[test]
    fn a_linked_label_beside_a_linked_figure_has_no_marker() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Beginning Balance", 1318609]]})).unwrap();
        s.execute("source.set", json!({"range": "A1:B1", "links": [[link("label"), link("balance")]]})).unwrap();
        assert!(!state(&s, "A1"), "the label");
        assert!(state(&s, "B1"), "the figure");
        assert_eq!(shown(&s, "A1:B1"), ["B1"]);
        // The link is still stored and read back.
        assert_eq!(s.execute("source.get", json!({"cell": "A1"})).unwrap()["link"]["locator"], "label");
    }

    #[test]
    fn a_formula_over_linked_cells_has_no_marker_beside_a_sourced_value() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100, 200]]})).unwrap();
        s.execute("source.set", json!({"range": "A1:B1", "links": [[link("a1"), link("b1")]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=A1+B1"})).unwrap();
        assert!(!state(&s, "C1"));
        assert!(state(&s, "B1"));
        assert_eq!(shown(&s, "A1:C1"), ["A1", "B1"]);
    }

    #[test]
    fn spilled_values_and_linked_formulas_have_no_marker() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B2", "values": [[100], [5]]})).unwrap();
        s.execute("source.set", json!({"range": "B2:B3", "links": [[link("b2")], [link("b3")]]})).unwrap();
        // D2 holds no formula of its own: it is spilled by D1.
        s.execute("cell.set", json!({"cell": "D1", "input": "=B2:B3*1"})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::parse("D2").unwrap()), gridcraft_core::Value::Number(5.0));
        // A formula cell takes no link.
        assert!(s.execute("source.set", json!({"range": "D1", "links": [[link("d1")]]})).is_err());
        assert!(!state(&s, "D1"));
        assert!(!state(&s, "D2"));
        assert!(shown(&s, "D1:D3").is_empty());
    }

    #[test]
    fn markers_change_with_edits_and_undo() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("opening")]]})).unwrap();
        assert!(state(&s, "A1"));
        s.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
        assert!(!state(&s, "A1"), "typing over it dropped its link");
        s.execute("range.setValues", json!({"range": "B1", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "B1", "links": [[link("other")]]})).unwrap();
        assert!(state(&s, "B1"));
        assert!(!state(&s, "A1"), "an unlinked number stays unmarked beside a link");
        s.execute("cell.set", json!({"cell": "A1", "input": "=B1"})).unwrap();
        assert!(!state(&s, "A1"), "a formula typed over it");
        for _ in 0..4 {
            s.execute("edit.undo", json!({})).unwrap();
        }
        assert!(state(&s, "A1"));
        assert!(!state(&s, "B1"));
    }

    #[test]
    fn bad_sheets_and_inverted_ranges_have_no_markers() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("a1")]]})).unwrap();
        let wb = &s.doc().unwrap().wb;
        let a1 = CellRef::parse("A1").unwrap();
        assert!(!is_sourced(wb, 7, a1));
        assert!(sourced_in(wb, 7, RangeRef::cell(a1)).is_empty());
        let inverted = RangeRef { start: CellRef::new(5, 5), end: CellRef::new(0, 0) };
        assert!(sourced_in(wb, 0, inverted).is_empty());
        let inverted_cols = RangeRef { start: CellRef::new(0, 5), end: CellRef::new(5, 0) };
        assert!(sourced_in(wb, 0, inverted_cols).is_empty());
        assert_eq!(sourced_in(wb, 0, RangeRef::cell(a1)), [a1]);
    }
}
