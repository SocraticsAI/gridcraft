// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Where a cell's value comes from. A hard-coded value carries at most one source link (the
//! document it was read from); a formula has none of its own and is traced through the cells it
//! reads (across sheets, 3-D references, defined names and table references) to hard-coded
//! values. [`trace`] answers for one cell.
//!
//! [`source_state`] and [`source_states_in`] give cells their marker state, computed once per
//! version of a document's workbook and cached on the document ([`DocState::source_states`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use gridcraft_calc::Key;
use gridcraft_calc::eval::{Area, Ev, Evaluator, Host};
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_formula::{Expr, StructItem};
use gridcraft_model::{DefinedName, Formula, Sheet, SourceLink, Workbook};

use crate::DocState;

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

/// A cell's marker: its value is traced to sources (blue) or not (amber). Cells without one get no marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    Sourced,
    Unsourced,
}

/// What a value rests on: some linked value, some unlinked number (or INDIRECT/OFFSET), or
/// (`unknown`) more than the work budget allowed to look at.
#[derive(Clone, Copy, Debug, Default)]
struct Flags {
    link: bool,
    unsourced: bool,
    unknown: bool,
}

impl Flags {
    fn merge(&mut self, o: Flags) {
        self.link |= o.link;
        self.unsourced |= o.unsourced;
        self.unknown |= o.unknown;
    }

    /// Unsourced when something unlinked was reached or not every input could be looked at;
    /// else Sourced when a link was reached; else no marker (text, empty, `=1+2`).
    fn state(self) -> Option<SourceState> {
        if self.unsourced || self.unknown {
            Some(SourceState::Unsourced)
        } else if self.link {
            Some(SourceState::Sourced)
        } else {
            None
        }
    }
}

/// Work one workbook version may spend on markers: each stored row, cell and spilled value
/// scanned in a range, each reference read, each defined name expanded. Past it, formulas not yet
/// worked out are Unsourced (not every input can be confirmed), rather than the grid stalling on
/// a pathological workbook.
const STATE_WORK_CAP: usize = 2_000_000;

/// What the memo works states out for: a cell, or a range a formula reads (worked out once
/// however many formulas read it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Cell(Key),
    Area(Area),
}

/// A node whose inputs are being visited.
#[derive(Debug)]
struct Frame {
    node: Node,
    inputs: Vec<Node>,
    acc: Flags,
    /// Tarjan's visit order and low link.
    index: usize,
    low: usize,
}

enum Start {
    /// A node that reads nothing: its flags.
    Leaf(Flags),
    /// A node's inputs, and the flags it has of its own.
    Inner(Vec<Node>, Flags),
}

/// Flags of every node worked out so far, for one workbook version.
struct Memo {
    done: HashMap<Node, Flags>,
    budget: Budget,
}

impl std::fmt::Debug for Memo {
    // A summary: a document printed for debugging shouldn't list up to millions of cells.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memo").field("nodes", &self.done.len()).field("budget_left", &self.budget.left).finish()
    }
}

impl Default for Memo {
    fn default() -> Memo {
        Memo::with_cap(STATE_WORK_CAP)
    }
}

impl Memo {
    fn with_cap(cap: usize) -> Memo {
        Memo { done: HashMap::new(), budget: Budget::new(cap) }
    }

    fn start(&mut self, wb: &Workbook, node: Node) -> Start {
        match node {
            Node::Cell(k) => self.start_cell(wb, k),
            Node::Area(a) => self.start_area(wb, a),
        }
    }

    /// A cell's own flags and the cells and ranges it reads.
    fn start_cell(&mut self, wb: &Workbook, (si, c): Key) -> Start {
        let Some(sh) = wb.sheet(si) else { return Start::Leaf(Flags::default()) };
        let mut acc = Flags::default();
        let areas = match classify(sh, c) {
            CellKind::Linked(_) => return Start::Leaf(Flags { link: true, ..Flags::default() }),
            CellKind::Number => return Start::Leaf(Flags { unsourced: true, ..Flags::default() }),
            CellKind::Other => return Start::Leaf(Flags::default()),
            CellKind::Spilled(anchor) => vec![Area { sheet: si, range: RangeRef::cell(anchor) }],
            CellKind::Formula(f) => {
                let Some(e) = f.expr_arc() else { return Start::Leaf(Flags::default()) };
                // Names are expanded once per formula, against the version's budget.
                let mut resolver = InputResolver::new(wb);
                let mut areas = Vec::new();
                resolver.formula_inputs(&mut self.budget, si, c, &e, 0, &mut areas);
                acc.unsourced = resolver.untraceable;
                acc.unknown = resolver.truncated;
                areas
            }
        };
        let mut inputs = Vec::with_capacity(areas.len());
        for a in areas {
            if !self.budget.take() {
                acc.unknown = true;
                break;
            }
            inputs.push(if a.range.start == a.range.end { Node::Cell((a.sheet, a.range.start)) } else { Node::Area(a) });
        }
        Start::Inner(inputs, acc)
    }

    /// The cells of a range that hold something (stored cells, then spilled values).
    fn start_area(&mut self, wb: &Workbook, area: Area) -> Start {
        let Some(sh) = wb.sheet(area.sheet) else { return Start::Leaf(Flags::default()) };
        if is_inverted(area.range) {
            return Start::Leaf(Flags::default());
        }
        let mut inputs = Vec::new();
        let complete = self.scan_area(sh, area, &mut inputs);
        Start::Inner(inputs, Flags { unknown: !complete, ..Flags::default() })
    }

    /// Adds the cells of `area` (on `sh`) that hold something to `out`, counting every stored row,
    /// cell and spilled value scanned; false when the budget ran out first.
    fn scan_area(&mut self, sh: &Sheet, area: Area, out: &mut Vec<Node>) -> bool {
        let r = area.range;
        for row in sh.cells.rows_in(r.start.row, r.end.row) {
            if !self.budget.take() {
                return false;
            }
            for (col, _) in sh.cells.row(row, r.start.col, r.end.col) {
                if !self.budget.take() {
                    return false;
                }
                let c = CellRef::new(row, col);
                if !is_blank(sh, c) {
                    out.push(Node::Cell((area.sheet, c)));
                }
            }
        }
        for (&c, _) in sh.spill.range(CellRef::new(r.start.row, 0)..=CellRef::new(r.end.row, u32::MAX)) {
            if !self.budget.take() {
                return false;
            }
            if r.contains(c) && is_blank(sh, c) {
                out.push(Node::Cell((area.sheet, c)));
            }
        }
        true
    }

    /// Flags of `root`: depth-first over its inputs with an explicit stack (no recursion), each
    /// node once per version. The nodes of a circular reference (a strongly connected component,
    /// found with Tarjan's algorithm) all get the flags of the whole cycle, whichever is asked first.
    fn flags(&mut self, wb: &Workbook, root: Node) -> Flags {
        if let Some(f) = self.done.get(&root) {
            return *f;
        }
        let (inputs, acc) = match self.start(wb, root) {
            Start::Leaf(f) => {
                self.done.insert(root, f);
                return f;
            }
            Start::Inner(inputs, acc) => (inputs, acc),
        };
        // Nodes of components still open, with their visit order.
        let mut open: HashMap<Node, usize> = HashMap::from([(root, 0)]);
        // Tarjan's stack: the nodes of components still open, in visit order.
        let mut members: Vec<Node> = vec![root];
        let mut stack = vec![Frame { node: root, inputs, acc, index: 0, low: 0 }];
        let mut next_index = 1;
        while let Some(top) = stack.last_mut() {
            if let Some(input) = top.inputs.pop() {
                if let Some(f) = self.done.get(&input) {
                    top.acc.merge(*f);
                } else if let Some(&i) = open.get(&input) {
                    // A circular reference: the input's flags join this node's at the component's root.
                    top.low = top.low.min(i);
                } else {
                    match self.start(wb, input) {
                        Start::Leaf(f) => {
                            self.done.insert(input, f);
                            top.acc.merge(f);
                        }
                        Start::Inner(inputs, acc) => {
                            open.insert(input, next_index);
                            members.push(input);
                            stack.push(Frame { node: input, inputs, acc, index: next_index, low: next_index });
                            next_index += 1;
                        }
                    }
                }
                continue;
            }
            let Some(frame) = stack.pop() else { break };
            if frame.low < frame.index {
                // Inside a component whose root is further up: its flags reach the root through
                // its parent, which is in the same component.
                if let Some(parent) = stack.last_mut() {
                    parent.low = parent.low.min(frame.low);
                    parent.acc.merge(frame.acc);
                }
                continue;
            }
            // The root of a component: every node in it rests on what the whole cycle rests on.
            while let Some(m) = members.pop() {
                open.remove(&m);
                self.done.insert(m, frame.acc);
                if m == frame.node {
                    break;
                }
            }
            if let Some(parent) = stack.last_mut() {
                parent.acc.merge(frame.acc);
            }
        }
        self.done.get(&root).copied().unwrap_or_default()
    }
}

/// Marker states for one version of one document's workbook. Kept on [`DocState`]; rebuilt when
/// the document's uid or revision changes (not when only its view state does).
#[derive(Debug, Default)]
pub struct SourceStateCache {
    inner: Mutex<CacheSlot>,
}

#[derive(Debug, Default)]
struct CacheSlot {
    entry: Option<CacheEntry>,
    /// Times the states were started afresh.
    #[cfg(test)]
    builds: usize,
}

#[derive(Debug)]
struct CacheEntry {
    uid: u64,
    revision: u64,
    any_links: bool,
    memo: Memo,
}

impl SourceStateCache {
    #[cfg(test)]
    fn builds(&self) -> usize {
        self.inner.lock().map(|slot| slot.builds).unwrap_or_default()
    }
}

/// Runs `f` on the memo for `d`'s current version; `None` when no sheet has a link.
fn with_memo<R>(d: &DocState, f: impl FnOnce(&mut Memo) -> R) -> Option<R> {
    let cache = &d.source_states.inner;
    let mut slot = match cache.lock() {
        Ok(slot) => slot,
        Err(poisoned) => {
            // A panic while working states out: start this version over.
            let mut slot = poisoned.into_inner();
            slot.entry = None;
            cache.clear_poison();
            slot
        }
    };
    let fresh = slot.entry.as_ref().is_some_and(|e| e.uid == d.uid && e.revision == d.revision);
    if !fresh {
        slot.entry = Some(CacheEntry {
            uid: d.uid,
            revision: d.revision,
            any_links: d.wb.sheets.iter().any(|s| !s.sources.is_empty()),
            memo: Memo::default(),
        });
        #[cfg(test)]
        {
            slot.builds += 1;
        }
    }
    let entry = slot.entry.as_mut()?;
    entry.any_links.then(|| f(&mut entry.memo))
}

/// The marker of one cell (`None`: no marker).
pub fn source_state(d: &DocState, sheet_index: usize, cell: CellRef) -> Option<SourceState> {
    d.wb.sheet(sheet_index)?;
    with_memo(d, |m| m.flags(&d.wb, Node::Cell((sheet_index, cell))).state()).flatten()
}

/// The markers of the cells of `range` that have one, row by row (what the grid paints for a
/// visible pane).
pub fn source_states_in(d: &DocState, sheet_index: usize, range: RangeRef) -> Vec<(CellRef, SourceState)> {
    let Some(sh) = d.wb.sheet(sheet_index) else { return Vec::new() };
    if is_inverted(range) {
        return Vec::new();
    }
    let mut out = with_memo(d, |m| {
        let spilled = sh
            .spill
            .range(CellRef::new(range.start.row, 0)..=CellRef::new(range.end.row, u32::MAX))
            .map(|(c, _)| *c)
            .filter(|c| range.contains(*c) && sh.cell(*c).is_none());
        let cells: Vec<CellRef> = sh.cells.iter_range(range).map(|(c, _)| c).chain(spilled).collect();
        cells.into_iter().filter_map(|c| Some((c, m.flags(&d.wb, Node::Cell((sheet_index, c))).state()?))).collect::<Vec<_>>()
    })
    .unwrap_or_default();
    out.sort_unstable_by_key(|(c, _)| *c);
    out
}

#[cfg(test)]
mod tests {
    use gridcraft_core::{CellRef, RangeRef};
    use serde_json::{Value as Json, json};

    use std::sync::Arc;

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

    fn state(s: &Session, cell: &str) -> Option<SourceState> {
        let d = s.doc().unwrap();
        source_state(d, d.wb.active_sheet, CellRef::parse(cell).unwrap())
    }

    fn builds(s: &Session) -> usize {
        s.doc().unwrap().source_states.builds()
    }

    #[test]
    fn a_workbook_without_links_has_no_markers() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2]]})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=A1+B1"})).unwrap();
        for c in ["A1", "B1", "C1"] {
            assert_eq!(state(&s, c), None, "{c}");
        }
        let d = s.doc().unwrap();
        assert!(source_states_in(d, 0, RangeRef::parse("A1:C1").unwrap()).is_empty());
    }

    #[test]
    fn markers_follow_links_through_formulas() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100, 7, "text"]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("opening")]]})).unwrap();
        for (cell, input) in [("D1", "=A1*2"), ("E1", "=A1+B1"), ("F1", "=UPPER(C1)"), ("G1", "=INDIRECT(\"A1\")"), ("H1", "=D1"), ("I1", "=1+2")] {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        assert_eq!(state(&s, "A1"), Some(SourceState::Sourced));
        assert_eq!(state(&s, "B1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s, "C1"), None);
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
        assert_eq!(state(&s, "E1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s, "F1"), None);
        assert_eq!(state(&s, "G1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s, "H1"), Some(SourceState::Sourced));
        assert_eq!(state(&s, "I1"), None);
        assert_eq!(state(&s, "Z9"), None);
        let d = s.doc().unwrap();
        let shown: Vec<String> = source_states_in(d, 0, RangeRef::parse("A1:I1").unwrap()).iter().map(|(c, _)| c.a1()).collect();
        assert_eq!(shown, ["A1", "B1", "D1", "E1", "G1", "H1"]);
    }

    #[test]
    fn markers_change_with_edits_and_undo() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("opening")]]})).unwrap();
        s.execute("cell.set", json!({"cell": "D1", "input": "=A1"})).unwrap();
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
        s.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
        assert_eq!(state(&s, "A1"), None, "the only link is gone, so no markers at all");
        s.execute("range.setValues", json!({"range": "B1", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "B1", "links": [[link("other")]]})).unwrap();
        assert_eq!(state(&s, "A1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s, "D1"), Some(SourceState::Unsourced));
        for _ in 0..3 {
            s.execute("edit.undo", json!({})).unwrap();
        }
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
    }

    #[test]
    fn circular_references_and_long_ranges_finish() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "C1", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "C1", "links": [[link("c1")]]})).unwrap();
        s.execute("cell.set", json!({"cell": "A1", "input": "=B1+C1"})).unwrap();
        s.execute("cell.set", json!({"cell": "B1", "input": "=A1+1"})).unwrap();
        assert_eq!(state(&s, "A1"), Some(SourceState::Sourced));
        let rows: Vec<Json> = (0..20_000).map(|i| json!([i])).collect();
        s.execute("range.setValues", json!({"range": "E1", "values": rows})).unwrap();
        s.execute("cell.set", json!({"cell": "F1", "input": "=SUM(E1:E20000)"})).unwrap();
        assert_eq!(state(&s, "F1"), Some(SourceState::Unsourced));
    }

    #[test]
    fn the_cells_of_a_circular_reference_share_one_state_whichever_is_asked_first() {
        // A1 reads B1 and the unlinked D1; B1 reads only A1. Both rest on D1.
        let build = || {
            let mut s = s();
            s.execute("range.setValues", json!({"range": "C1", "values": [[1, 5]]})).unwrap();
            s.execute("source.set", json!({"range": "C1", "links": [[link("c1")]]})).unwrap();
            s.execute("cell.set", json!({"cell": "A1", "input": "=B1+D1"})).unwrap();
            s.execute("cell.set", json!({"cell": "B1", "input": "=A1"})).unwrap();
            s
        };
        let s1 = build();
        assert_eq!(state(&s1, "A1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s1, "B1"), Some(SourceState::Unsourced));
        let s2 = build();
        assert_eq!(state(&s2, "B1"), Some(SourceState::Unsourced));
        assert_eq!(state(&s2, "A1"), Some(SourceState::Unsourced));
    }

    #[test]
    fn defined_names_spills_and_tables_carry_states() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B2", "values": [[100], [5]]})).unwrap();
        s.execute("source.set", json!({"range": "B2:B3", "links": [[link("b2")], [link("b3")]]})).unwrap();
        s.execute("formulas.defineName", json!({"name": "Opening", "refersTo": "=Sheet1!$B$2"})).unwrap();
        s.execute("formulas.defineName", json!({"name": "Twice", "refersTo": "=Opening*2"})).unwrap();
        s.execute("cell.set", json!({"cell": "E1", "input": "=Twice+1"})).unwrap();
        assert_eq!(state(&s, "E1"), Some(SourceState::Sourced));
        // D2 holds no formula of its own: it is spilled by D1.
        s.execute("cell.set", json!({"cell": "D1", "input": "=B2:B3*1"})).unwrap();
        assert_eq!(state(&s, "D2"), Some(SourceState::Sourced));
        let d = s.doc().unwrap();
        let shown: Vec<String> = source_states_in(d, 0, RangeRef::parse("D1:D2").unwrap()).iter().map(|(c, _)| c.a1()).collect();
        assert_eq!(shown, ["D1", "D2"]);
        s.execute("range.setValues", json!({"range": "A10", "values": [["Item", "Amount"], ["Rent", 1200], ["Fees", 300]]})).unwrap();
        s.execute("source.set", json!({"range": "B11", "links": [[link("rent")]]})).unwrap();
        s.execute("insert.table", json!({"range": "A10:B12", "header": true, "name": "Costs"})).unwrap();
        s.execute("cell.set", json!({"cell": "D10", "input": "=SUM(Costs[Amount])"})).unwrap();
        assert_eq!(state(&s, "D10"), Some(SourceState::Unsourced), "B12 has no link");
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn names_that_fan_out_are_expanded_once_each() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B2", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "B2", "links": [[link("b2")]]})).unwrap();
        for k in 1..16 {
            let next = format!("N_{}", k + 1);
            s.execute("formulas.defineName", json!({"name": format!("N_{k}"), "refersTo": format!("={next}+{next}+{next}+{next}")})).unwrap();
        }
        s.execute("formulas.defineName", json!({"name": "N_16", "refersTo": "=Sheet1!$B$2"})).unwrap();
        // Written straight into the workbook: `cell.set` would compute it (the evaluator has no guard).
        let formula = gridcraft_model::Cell { formula: Some(Arc::new(Formula::new("=N_1"))), ..Default::default() };
        let d = s.doc_mut().unwrap();
        Arc::make_mut(&mut d.wb).sheet_mut(0).unwrap().set_cell(CellRef::parse("E1").unwrap(), formula);
        d.revision += 1;
        let started = std::time::Instant::now();
        assert_eq!(state(&s, "E1"), Some(SourceState::Sourced));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn ranges_over_many_stored_blanks_stay_bounded() {
        // 200,000 stored blank cells in B (formatted, never typed in) above a linked number, and
        // 5,000 formulas each reading column B. Written straight into the workbook: `cell.set`
        // would recalculate every SUM.
        let mut s = s();
        s.execute("range.setValues", json!({"range": "B200001", "values": [[7]]})).unwrap();
        s.execute("source.set", json!({"range": "B200001", "links": [[link("b")]]})).unwrap();
        let at = |a1: &str| CellRef::parse(a1).unwrap();
        let formula = |text: &str| gridcraft_model::Cell { formula: Some(Arc::new(Formula::new(text))), ..Default::default() };
        let d = s.doc_mut().unwrap();
        let sh = Arc::make_mut(&mut d.wb).sheet_mut(0).unwrap();
        let blank = gridcraft_model::Cell { style: gridcraft_model::StyleId(1), ..Default::default() };
        for row in 0..200_000 {
            sh.set_cell(CellRef::new(row, 1), blank.clone());
        }
        for k in 1..=5_000 {
            // The same area over and over: scanned once.
            sh.set_cell(at(&format!("D{k}")), formula(&format!("=SUM(B:B)+D{}", k + 1)));
            // A different area each time: the work budget runs out.
            sh.set_cell(at(&format!("E{k}")), formula(&format!("=SUM(B{k}:B1048576)+E{}", k + 1)));
        }
        d.revision += 1;
        let started = std::time::Instant::now();
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
        assert_eq!(state(&s, "E1"), Some(SourceState::Unsourced), "past the budget a formula can't be confirmed");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn a_memo_out_of_budget_marks_formulas_unsourced_and_values_as_they_are() {
        let mut s = s();
        let rows: Vec<Json> = (0..20).map(|i| json!([i])).collect();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        let links: Vec<Json> = (0..20).map(|i| json!([link(&format!("a{i}"))])).collect();
        s.execute("source.set", json!({"range": "A1:A20", "links": links})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "=SUM(A1:A20)"})).unwrap();
        let wb = &s.doc().unwrap().wb;
        let c1 = Node::Cell((0, CellRef::parse("C1").unwrap()));
        assert_eq!(Memo::with_cap(100).flags(wb, c1).state(), Some(SourceState::Sourced));
        let mut small = Memo::with_cap(5);
        assert_eq!(small.flags(wb, c1).state(), Some(SourceState::Unsourced));
        assert_eq!(small.flags(wb, Node::Cell((0, CellRef::parse("A3").unwrap()))).state(), Some(SourceState::Sourced));
    }

    #[test]
    fn the_cache_is_kept_across_view_changes_and_rebuilt_by_edits_undo_redo_and_recalc() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[100]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("opening")]]})).unwrap();
        s.execute("cell.set", json!({"cell": "D1", "input": "=A1"})).unwrap();
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
        let d = s.doc().unwrap();
        assert_eq!(source_states_in(d, 0, RangeRef::parse("A1:D1").unwrap()).len(), 2);
        assert_eq!(builds(&s), 1);
        // Moving the cursor and storing the view (as the grid does every frame) changes the
        // workbook's address while another handle holds it, but not its content: the states are kept.
        s.execute("selection.set", json!({"range": "F9"})).unwrap();
        let held = s.doc().unwrap().wb.clone();
        s.active_mut().unwrap().store_view(&[(0, CellRef::parse("B5").unwrap())]);
        assert!(!Arc::ptr_eq(&held, &s.doc().unwrap().wb), "store_view made a new workbook");
        drop(held);
        assert_eq!(state(&s, "D1"), Some(SourceState::Sourced));
        assert_eq!(builds(&s), 1);
        s.execute("cell.set", json!({"cell": "E1", "input": "3"})).unwrap();
        assert_eq!(state(&s, "E1"), Some(SourceState::Unsourced));
        assert_eq!(builds(&s), 2);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(state(&s, "E1"), None);
        assert_eq!(builds(&s), 3);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(state(&s, "E1"), Some(SourceState::Unsourced));
        assert_eq!(builds(&s), 4);
        s.execute("formulas.calculateNow", json!({})).unwrap();
        assert_eq!(state(&s, "E1"), Some(SourceState::Unsourced));
        assert_eq!(builds(&s), 5, "a recalculation can move spilled values");
    }

    #[test]
    fn bad_sheets_and_inverted_ranges_have_no_markers() {
        let mut s = s();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1]]})).unwrap();
        s.execute("source.set", json!({"range": "A1", "links": [[link("a1")]]})).unwrap();
        let d = s.doc().unwrap();
        let a1 = CellRef::parse("A1").unwrap();
        assert_eq!(source_state(d, 7, a1), None);
        assert!(source_states_in(d, 7, RangeRef::cell(a1)).is_empty());
        let inverted = RangeRef { start: CellRef::new(5, 5), end: CellRef::new(0, 0) };
        assert!(source_states_in(d, 0, inverted).is_empty());
        let inverted_cols = RangeRef { start: CellRef::new(0, 5), end: CellRef::new(5, 0) };
        assert!(source_states_in(d, 0, inverted_cols).is_empty());
        assert_eq!(source_states_in(d, 0, RangeRef::cell(a1)), [(a1, SourceState::Sourced)]);
    }
}
