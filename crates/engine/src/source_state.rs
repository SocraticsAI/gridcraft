// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Where a cell's value comes from. A hard-coded value carries at most one source link (the
//! document it was read from); a formula has none of its own and is traced through the cells it
//! reads (across sheets, 3-D references, defined names and table references) to hard-coded
//! values. [`trace`] answers for one cell.

use std::collections::{HashSet, VecDeque};

use gridcraft_calc::Key;
use gridcraft_calc::eval::{Area, Ev, Evaluator, Host};
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_formula::Expr;
use gridcraft_model::{DefinedName, Formula, Sheet, SourceLink, Workbook};

/// Most steps one trace takes: each cell visited, each defined name expanded and each reference a
/// name expands to counts one. Past it the trace reports `truncated`.
pub const TRACE_CAP: usize = 10_000;
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

/// The defined name `n` (maybe `Sheet1!Name`) as seen from a formula on `sheet`.
fn defined_name<'w>(wb: &'w Workbook, sheet: usize, n: &str) -> Option<&'w DefinedName> {
    let (scope_sheet, bare) = match n.split_once('!') {
        Some((s, b)) => (wb.sheet_index(s.trim_matches('\''))?, b),
        None => (sheet, n),
    };
    wb.name(bare, scope_sheet)
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
/// unqualified references point at, and the cell its relative references move by (`None`: it
/// has none, so every use expands the same).
type NameUse = (Option<usize>, String, usize, Option<CellRef>);

/// One trace in progress.
struct Tracer<'a> {
    wb: &'a Workbook,
    out: SourceTrace,
    /// Cells already queued, each visited once (also ends cycles).
    seen: HashSet<Key>,
    /// Defined names already expanded in this trace.
    names: HashSet<NameUse>,
    /// Steps taken, against [`TRACE_CAP`].
    spent: usize,
}

impl Tracer<'_> {
    /// Takes one step of the budget; false (and `truncated`) once it is used up.
    fn spend(&mut self) -> bool {
        if self.spent >= TRACE_CAP {
            self.out.truncated = true;
            return false;
        }
        self.spent += 1;
        true
    }

    /// Adds the areas formula `e` (in `sheet`, at `at`) reads to `out`, with defined names and
    /// table references resolved. `depth` counts the defined names this formula sits inside.
    fn formula_inputs(&mut self, sheet: usize, at: CellRef, e: &Expr, depth: usize, out: &mut Vec<Area>) {
        let wb = self.wb;
        let mut own: Vec<Area> = gridcraft_calc::eval::precedents(wb, sheet, e).0;
        let mut names: Vec<&str> = Vec::new();
        let mut called: Vec<&str> = Vec::new();
        let mut structs: Vec<&Expr> = Vec::new();
        e.walk(&mut |x| match x {
            Expr::Call(n, _) if matches!(n.to_ascii_uppercase().as_str(), "INDIRECT" | "OFFSET") => self.out.untraceable = true,
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
            while kept < own.len() && self.spend() {
                kept += 1;
            }
            own.truncate(kept);
        }
        out.extend(own);
        for n in defined {
            self.expand_name(sheet, at, n, depth, out);
        }
    }

    /// Adds the areas defined name `n`, used in a formula on `sheet` at `at`, reads — once per
    /// trace for each way it can expand.
    fn expand_name(&mut self, sheet: usize, at: CellRef, n: &str, depth: usize, out: &mut Vec<Area>) {
        let Some(def) = defined_name(self.wb, sheet, n) else { return };
        let Ok(expr) = gridcraft_formula::parse(def.formula.trim_start_matches('=')) else { return };
        let relative = gridcraft_formula::adjust::has_relative_refs(&expr);
        // Unqualified references in a name refer to its scope sheet, else the sheet using it.
        let scope = def.scope.unwrap_or(sheet);
        let key: NameUse = (def.scope, def.name.to_ascii_lowercase(), scope, relative.then_some(at));
        if self.names.contains(&key) {
            return;
        }
        if depth >= MAX_NAME_DEPTH {
            self.out.untraceable = true;
            return;
        }
        if !self.spend() {
            return;
        }
        self.names.insert(key);
        // Relative references in a name are relative to A1: used here, they move by this cell's offset.
        let expr = if relative { gridcraft_formula::adjust::shift_relative_wrapping(expr, i64::from(at.row), i64::from(at.col)) } else { expr };
        self.formula_inputs(scope, at, &expr, depth + 1, out);
    }

    /// Queues each cell of `area` that holds something (stored cells, then spilled values) and
    /// isn't queued yet, while the budget lasts.
    fn queue_area(&mut self, area: &Area, queue: &mut VecDeque<Key>) {
        let wb = self.wb;
        let Some(sh) = wb.sheet(area.sheet) else { return };
        let r = area.range;
        // Ranges are normalised; an inverted one would make the spill lookup below panic.
        if r.start.row > r.end.row {
            return;
        }
        let stored = sh.cells.iter_range(r).map(|(c, _)| c).filter(|c| !is_blank(sh, *c));
        let spilled = sh.spill.range(CellRef::new(r.start.row, 0)..=CellRef::new(r.end.row, u32::MAX)).map(|(c, _)| *c);
        for c in stored.chain(spilled.filter(|c| r.contains(*c) && is_blank(sh, *c))) {
            let k = (area.sheet, c);
            if self.seen.contains(&k) {
                continue;
            }
            if !self.spend() {
                return;
            }
            self.seen.insert(k);
            queue.push_back(k);
        }
    }
}

/// What `cell` on `sheet` rests on: its own link if it is a linked value, else the linked values
/// and unlinked numbers its formula reads, breadth-first, each once.
pub fn trace(wb: &Workbook, sheet: usize, cell: CellRef) -> SourceTrace {
    let mut t = Tracer { wb, out: SourceTrace::default(), seen: HashSet::from([(sheet, cell)]), names: HashSet::new(), spent: 1 };
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
                    t.formula_inputs(si, c, &e, 0, &mut inputs);
                }
            }
        }
        for area in &inputs {
            t.queue_area(area, &mut queue);
        }
    }
    t.out
}
