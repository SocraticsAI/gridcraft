// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Where a cell's value comes from. A hard-coded value carries at most one source link (the
//! document it was read from); a formula has none of its own and is traced through the cells it
//! reads (across sheets, 3-D references, defined names and table references) to hard-coded
//! values. [`trace`] answers for one cell.
//!
//! [`is_sourced`] and [`sourced_in`] say which cells carry a marker: hard-coded numbers with a
//! link, read from the cell itself. A formula is traced but never marked; an unlinked number is
//! reported by a trace (`unsourced`) but not marked either.

use std::collections::{HashSet, VecDeque};

use gridcraft_calc::Key;
use gridcraft_calc::eval::{Area, Ev, Evaluator, Host};
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_formula::{Expr, StructItem};
use gridcraft_model::{DefinedName, Formula, Sheet, SourceLink, Workbook};

/// Most steps one trace takes: each cell visited, each defined name expanded and each reference a
/// name expands to counts one. Past it the trace reports `truncated`.
pub const TRACE_CAP: usize = 10_000;
/// Most stored rows, cells and spilled values one trace scans while reading the ranges formulas
/// cover (blanks and cells already reached included). Past it the trace reports `truncated`.
const SCAN_CAP: usize = 100 * TRACE_CAP;
/// Longest chain of defined names followed (a name that refers to a name…).
const MAX_NAME_DEPTH: usize = 16;

/// A linked value reached by a trace.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedLink {
    pub sheet: usize,
    pub cell: CellRef,
    pub link: SourceLink,
}

/// What a cell's value rests on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceTrace {
    /// Linked values reached, each once.
    pub links: Vec<TracedLink>,
    /// Hard-coded numbers reached that have no link.
    pub unsourced: Vec<(usize, CellRef)>,
    /// A formula on the way uses INDIRECT or OFFSET (whose cells can't be known from the formula),
    /// or names nested deeper than the trace follows.
    pub untraceable: bool,
    /// The trace reached [`TRACE_CAP`]: the lists are partial.
    pub truncated: bool,
}

/// What one cell is, for tracing.
enum CellKind<'a> {
    Formula(&'a Formula),
    /// A value spilled by the dynamic array anchored at this cell.
    Spilled(CellRef),
    Linked(&'a SourceLink),
    Number,
    Other,
}

fn classify(sh: &Sheet, c: CellRef) -> CellKind<'_> {
    let stored = sh.cell(c);
    if let Some(f) = stored.and_then(|x| x.formula.as_ref()) {
        return CellKind::Formula(f);
    }
    match stored.map(|x| &x.value).filter(|v| !v.is_empty()) {
        Some(v) => match sh.sources.get(&c) {
            Some(link) => CellKind::Linked(link),
            None if v.is_number() => CellKind::Number,
            None => CellKind::Other,
        },
        None if sh.spill.contains_key(&c) => match spill_anchor(sh, c) {
            Some(anchor) if anchor != c => CellKind::Spilled(anchor),
            _ => CellKind::Other,
        },
        None => CellKind::Other,
    }
}

/// The anchor of the spill range covering `c`.
fn spill_anchor(sh: &Sheet, c: CellRef) -> Option<CellRef> {
    sh.spill_ranges.iter().find(|(_, r)| r.contains(c)).map(|(a, _)| *a)
}

/// Whether a stored cell holds nothing a trace could stop at or walk through (a formatted blank).
fn is_blank(sh: &Sheet, c: CellRef) -> bool {
    sh.cell(c).is_none_or(|x| x.formula.is_none() && x.value.is_empty())
}

/// Whether `r` runs backwards (normalised ranges never do; the store's range lookups would panic).
fn is_inverted(r: RangeRef) -> bool {
    r.start.row > r.end.row || r.start.col > r.end.col
}

/// The defined name `n` (maybe `Sheet1!Name`) as seen from a formula on `sheet`.
fn defined_name<'w>(wb: &'w Workbook, sheet: usize, n: &str) -> Option<&'w DefinedName> {
    let (scope_sheet, bare) = match n.split_once('!') {
        Some((s, b)) => (wb.sheet_index(s.trim_matches('\''))?, b),
        None => (sheet, n),
    };
    wb.name(bare, scope_sheet)
}

/// Whether `e` holds a structured reference whose cells depend on where it is used: this row
/// (`Costs[@Amount]`) or the enclosing table (`[Amount]`). The walk is bounded by the parser's
/// nesting limit.
fn has_positional_struct(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |x| {
        if let Expr::Struct(r) = x {
            found |= r.table.is_empty() || r.specifiers.contains(&StructItem::ThisRow);
        }
    });
    found
}

/// A host for resolving table names and structured references to the cells they cover.
struct RefHost<'a> {
    wb: &'a Workbook,
}

impl Host for RefHost<'_> {
    fn workbook(&self) -> &Workbook {
        self.wb
    }
    fn cell_value(&mut self, sheet: usize, c: CellRef) -> Value {
        self.wb.sheet(sheet).map(|sh| sh.value(c)).unwrap_or_default()
    }
    fn now_serial(&self) -> f64 {
        0.0
    }
    fn random(&mut self) -> f64 {
        0.0
    }
}

/// The areas a reference-valued expression (a table name, `Table[Column]`) covers.
fn reference_areas(wb: &Workbook, sheet: usize, at: CellRef, e: &Expr) -> Option<Vec<Area>> {
    let mut host = RefHost { wb };
    match Evaluator::new(&mut host, sheet, at).eval(e) {
        Ev::R(areas) => Some(areas),
        _ => None,
    }
}

/// A defined name as expanded at one place: the name (scope, lower-cased), the sheet its
/// unqualified references point at, and the cell its relative references and this-row structured
/// references depend on (`None`: it has none, so every use expands the same).
type NameUse = (Option<usize>, String, usize, Option<CellRef>);

/// A number of steps that may be taken, one at a time.
#[derive(Debug)]
struct Budget {
    left: usize,
}

impl Budget {
    fn new(cap: usize) -> Budget {
        Budget { left: cap }
    }

    /// Takes one step; false once none are left.
    fn take(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        true
    }
}

/// Works out the areas formulas read, resolving defined names (each way once, nested at most
/// [`MAX_NAME_DEPTH`] deep) and table references. Expanding names takes steps of a budget.
struct InputResolver<'a> {
    wb: &'a Workbook,
    /// Defined names already expanded.
    names: HashSet<NameUse>,
    /// A formula read uses INDIRECT or OFFSET, or names nested deeper than followed.
    untraceable: bool,
    /// The budget ran out while expanding names: the areas are partial.
    truncated: bool,
}

impl<'a> InputResolver<'a> {
    fn new(wb: &'a Workbook) -> InputResolver<'a> {
        InputResolver { wb, names: HashSet::new(), untraceable: false, truncated: false }
    }

    /// Takes one step of `budget`; false (and `truncated`) once it is used up.
    fn spend(&mut self, budget: &mut Budget) -> bool {
        let ok = budget.take();
        self.truncated |= !ok;
        ok
    }

    /// Adds the areas formula `e` (in `sheet`, at `at`) reads to `out`, with defined names and
    /// table references resolved. `depth` counts the defined names this formula sits inside.
    fn formula_inputs(&mut self, budget: &mut Budget, sheet: usize, at: CellRef, e: &Expr, depth: usize, out: &mut Vec<Area>) {
        let wb = self.wb;
        let mut own: Vec<Area> = gridcraft_calc::eval::precedents(wb, sheet, e).0;
        let mut names: Vec<&str> = Vec::new();
        let mut called: Vec<&str> = Vec::new();
        let mut structs: Vec<&Expr> = Vec::new();
        e.walk(&mut |x| match x {
            Expr::Call(n, _) if matches!(n.to_ascii_uppercase().as_str(), "INDIRECT" | "OFFSET") => self.untraceable = true,
            // A call of a LAMBDA defined as a name.
            Expr::Call(n, _) if !gridcraft_calc::is_known_function(n) => called.push(n.as_str()),
            Expr::Name(n) => names.push(n.as_str()),
            Expr::Struct(_) => structs.push(x),
            _ => {}
        });
        let mut defined: Vec<&str> = Vec::new();
        for n in names {
            if defined_name(wb, sheet, n).is_some() {
                defined.push(n);
            } else if let Some(areas) = reference_areas(wb, sheet, at, &Expr::Name(n.to_string())) {
                // A table name alone: its data body. LET/LAMBDA parameters resolve to nothing here.
                own.extend(areas);
            }
        }
        defined.extend(called.into_iter().filter(|n| defined_name(wb, sheet, n).is_some()));
        for s in structs {
            if let Some(areas) = reference_areas(wb, sheet, at, s) {
                own.extend(areas);
            }
        }
        // References a name expands to count against the budget; a cell's own are bounded by its
        // formula and counted as the cells they reach.
        if depth > 0 {
            let mut kept = 0;
            while kept < own.len() && self.spend(budget) {
                kept += 1;
            }
            own.truncate(kept);
        }
        out.extend(own);
        for n in defined {
            self.expand_name(budget, sheet, at, n, depth, out);
        }
    }

    /// Adds the areas defined name `n`, used in a formula on `sheet` at `at`, reads — once for
    /// each way it can expand.
    fn expand_name(&mut self, budget: &mut Budget, sheet: usize, at: CellRef, n: &str, depth: usize, out: &mut Vec<Area>) {
        let Some(def) = defined_name(self.wb, sheet, n) else { return };
        let Ok(expr) = gridcraft_formula::parse(def.formula.trim_start_matches('=')) else { return };
        let relative = gridcraft_formula::adjust::has_relative_refs(&expr);
        // Unqualified references in a name refer to its scope sheet, else the sheet using it.
        let scope = def.scope.unwrap_or(sheet);
        // A structured reference may name the using cell's row (`Costs[@Amount]`) or table: such
        // a name expands differently where it is used, like one with relative references.
        let key: NameUse = (def.scope, def.name.to_ascii_lowercase(), scope, (relative || has_positional_struct(&expr)).then_some(at));
        if self.names.contains(&key) {
            return;
        }
        if depth >= MAX_NAME_DEPTH {
            self.untraceable = true;
            return;
        }
        if !self.spend(budget) {
            return;
        }
        self.names.insert(key);
        // Relative references in a name are relative to A1: used here, they move by this cell's offset.
        let expr = if relative { gridcraft_formula::adjust::shift_relative_wrapping(expr, i64::from(at.row), i64::from(at.col)) } else { expr };
        self.formula_inputs(budget, scope, at, &expr, depth + 1, out);
    }
}

/// One trace in progress.
struct Tracer<'a> {
    wb: &'a Workbook,
    out: SourceTrace,
    /// Cells already queued, each visited once (also ends cycles).
    seen: HashSet<Key>,
    /// Resolves the areas formulas read; defined names already expanded in this trace.
    inputs: InputResolver<'a>,
    /// Steps, against [`TRACE_CAP`].
    steps: Budget,
    /// Areas already scanned: a range many formulas read is walked once.
    scanned: HashSet<Area>,
    /// Entries scanned, against [`SCAN_CAP`].
    scans: Budget,
}

impl Tracer<'_> {
    /// Takes one step of the budget; false (and `truncated`) once it is used up.
    fn spend(&mut self) -> bool {
        let ok = self.steps.take();
        self.out.truncated |= !ok;
        ok
    }

    /// Takes one entry of the scan budget; false (and `truncated`) once it is used up.
    fn scan(&mut self) -> bool {
        let ok = self.scans.take();
        self.out.truncated |= !ok;
        ok
    }

    /// Queues each cell of `area` that holds something (stored cells, then spilled values) and
    /// isn't queued yet, while the budgets last. Each area is scanned once per trace.
    fn queue_area(&mut self, area: &Area, queue: &mut VecDeque<Key>) {
        let wb = self.wb;
        let Some(sh) = wb.sheet(area.sheet) else { return };
        let r = area.range;
        if is_inverted(r) || !self.scanned.insert(*area) {
            return;
        }
        for row in sh.cells.rows_in(r.start.row, r.end.row) {
            if !self.scan() {
                return;
            }
            for (col, _) in sh.cells.row(row, r.start.col, r.end.col) {
                let c = CellRef::new(row, col);
                if !self.scan() || (!is_blank(sh, c) && !self.enqueue((area.sheet, c), queue)) {
                    return;
                }
            }
        }
        for (&c, _) in sh.spill.range(CellRef::new(r.start.row, 0)..=CellRef::new(r.end.row, u32::MAX)) {
            if !self.scan() || (r.contains(c) && is_blank(sh, c) && !self.enqueue((area.sheet, c), queue)) {
                return;
            }
        }
    }

    /// Queues `k` unless it is queued already; false once the step budget is used up.
    fn enqueue(&mut self, k: Key, queue: &mut VecDeque<Key>) -> bool {
        if self.seen.contains(&k) {
            return true;
        }
        if !self.spend() {
            return false;
        }
        self.seen.insert(k);
        queue.push_back(k);
        true
    }
}

/// What `cell` on `sheet` rests on: its own link if it is a linked value, else the linked values
/// and unlinked numbers its formula reads, breadth-first, each once.
pub fn trace(wb: &Workbook, sheet: usize, cell: CellRef) -> SourceTrace {
    let mut t = Tracer {
        wb,
        out: SourceTrace::default(),
        seen: HashSet::from([(sheet, cell)]),
        inputs: InputResolver::new(wb),
        // The traced cell itself is the first step.
        steps: Budget::new(TRACE_CAP - 1),
        scanned: HashSet::new(),
        scans: Budget::new(SCAN_CAP),
    };
    let mut queue: VecDeque<Key> = VecDeque::from([(sheet, cell)]);
    while let Some((si, c)) = queue.pop_front() {
        let Some(sh) = wb.sheet(si) else { continue };
        let mut inputs: Vec<Area> = Vec::new();
        match classify(sh, c) {
            CellKind::Linked(link) => t.out.links.push(TracedLink { sheet: si, cell: c, link: link.clone() }),
            CellKind::Number => t.out.unsourced.push((si, c)),
            CellKind::Other => {}
            CellKind::Spilled(anchor) => inputs.push(Area { sheet: si, range: RangeRef::cell(anchor) }),
            CellKind::Formula(f) => {
                // Cells already queued are still listed; past the cap nothing new is expanded.
                if !t.out.truncated
                    && let Some(e) = f.expr_arc()
                {
                    t.inputs.formula_inputs(&mut t.steps, si, c, &e, 0, &mut inputs);
                    t.out.untraceable |= t.inputs.untraceable;
                    t.out.truncated |= t.inputs.truncated;
                }
            }
        }
        for area in &inputs {
            t.queue_area(area, &mut queue);
        }
    }
    t.out
}

/// Whether `c` on `sh` shows as sourced, read from the cell alone: a hard-coded number with a
/// source link. Linked text and logicals (labels, headers), unlinked numbers, formulas (whatever
/// they read) and spilled values don't.
fn sourced_on(sh: &Sheet, c: CellRef) -> bool {
    matches!(classify(sh, c), CellKind::Linked(_)) && sh.cell(c).is_some_and(|x| x.value.is_number())
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
        json!({"file": "Data Room/Cash/Statement.pdf", "locator": locator, "storeRow": "S!C12"})
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
        // A link stored on a formula cell (not what a host writes) shows nothing either.
        s.execute("source.set", json!({"range": "D1", "links": [[link("d1")]]})).unwrap();
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
