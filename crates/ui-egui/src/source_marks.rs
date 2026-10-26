// Added by Socratics, 2026. See MODIFICATIONS.md.
//! The Source Inspector (the Sources toggle in the sheet-tab bar, on by default): a cell source
//! marker, a tiny square (`Tokens::sourced_mark`) in the top-left corner of a hard-coded number
//! that has a source link, with a tooltip naming the source; and for an active cell holding a
//! linked number, a tint over the cell (`Tokens::source_selected_fill`). A plain click on a linked
//! number goes to the host (`linked_at`), which shows its source. Unlinked numbers, text and
//! formulas show nothing. Normal view only: Page Layout view shows nothing.

use egui::{Painter, Pos2, Rect, vec2};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::{Sheet, SourceLink, Workbook};
use gridcraft_engine::source_state::{is_sourced, sourced_in};
use gridcraft_engine::{DocState, Selection};

use crate::SheetApp;
use crate::grid::Geo;
use crate::theme::Tokens;

/// Side of the painted square at zoom `z` (the size of the note indicator).
pub fn marker_size(z: f32) -> f32 {
    6.0 * z.max(0.7)
}

/// The cell's rectangle, or its merged area's when `c` anchors one; `None` inside a merge.
fn cell_area(geo: &Geo, sh: &Sheet, c: CellRef) -> Option<Rect> {
    match sh.merges.iter().find(|m| m.contains(c)) {
        Some(m) if m.start == c => Some(geo.range_rect(sh, *m)),
        Some(_) => None,
        None => Some(geo.cell_rect(sh, c)),
    }
}

/// Where hovering counts as on the marker of `c` (at least 8 points square), in logical
/// (left-to-right) coordinates.
pub fn marker_rect(geo: &Geo, sh: &Sheet, c: CellRef) -> Option<Rect> {
    let r = cell_area(geo, sh, c)?;
    let s = marker_size(geo.z).max(8.0);
    Some(Rect::from_min_size(r.left_top(), vec2(s, s)))
}

/// Paints the markers of one pane (rows × cols). States are read from the cells.
#[allow(clippy::too_many_arguments)]
pub fn paint_quadrant(d: &DocState, p: &Painter, geo: &Geo, si: usize, sh: &Sheet, rows: (u32, u32), cols: (u32, u32), t: &Tokens) {
    if geo.paper.is_some() || rows.0 > rows.1 || cols.0 > cols.1 {
        return;
    }
    let range = RangeRef::new(CellRef::new(rows.0, cols.0), CellRef::new(rows.1, cols.1));
    let s = marker_size(geo.z);
    for c in sourced_in(&d.wb, si, range) {
        if sh.row_height(c.row) == 0.0 || sh.col_width(c.col) == 0.0 {
            continue;
        }
        let Some(r) = cell_area(geo, sh, c) else { continue };
        p.rect_filled(Rect::from_min_size(r.left_top(), vec2(s, s)), 0.0, t.sourced_mark);
    }
}

/// The cell whose marker is under `p` (logical coordinates), if any. Normal view only.
pub fn hit(geo: &Geo, d: &DocState, si: usize, sh: &Sheet, p: Pos2) -> Option<CellRef> {
    if geo.paper.is_some() || !geo.in_cells(p) {
        return None;
    }
    let c = geo.cell_at(sh, p);
    let anchor = sh.merges.iter().find(|m| m.contains(c)).map_or(c, |m| m.start);
    if !is_sourced(&d.wb, si, anchor) {
        return None;
    }
    marker_rect(geo, sh, anchor)?.contains(p).then_some(anchor)
}

/// The active cell (its merged area's anchor) when the Source Inspector shows it as linked: the
/// inspector is on, the view is Normal, no cell is being edited and it holds a linked hard-coded
/// number. It gets the tint.
pub fn active_linked(app: &SheetApp, geo: &Geo, si: usize, sh: &Sheet, sel: &Selection) -> Option<CellRef> {
    if !app.ui.source_inspector || geo.paper.is_some() || app.editor.is_some() {
        return None;
    }
    let d = app.session.active()?;
    let anchor = sh.merge_at(sel.active).map_or(sel.active, |m| m.start);
    is_sourced(&d.wb, si, anchor).then_some(anchor)
}

/// `c` (its merged area's anchor) when a click on it should show its source: the inspector is on,
/// the view is Normal, no cell is being edited and it holds a linked hard-coded number.
pub fn linked_at(app: &SheetApp, geo: &Geo, si: usize, sh: &Sheet, c: CellRef) -> Option<CellRef> {
    if !app.ui.source_inspector || geo.paper.is_some() || app.editor.is_some() {
        return None;
    }
    let d = app.session.active()?;
    let anchor = sh.merge_at(c).map_or(c, |m| m.start);
    is_sourced(&d.wb, si, anchor).then_some(anchor)
}

/// `Heritage Trust Bank Statement September 2019 · Beginning Balance`: the document's name
/// without folder or extension, and the last part of the locator.
pub fn label(link: &SourceLink) -> String {
    let name = link.file.rsplit(['/', '\\']).next().unwrap_or(&link.file);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).replace('_', " ");
    let place = link.locator.rsplit(" / ").next().unwrap_or(&link.locator).trim();
    if place.is_empty() { stem } else { format!("{stem} · {place}") }
}

/// The tooltip of the marker of `c`: its source. `None` for a cell without a
/// marker (an unlinked number, text, a formula).
pub fn tip(wb: &Workbook, si: usize, c: CellRef) -> Option<String> {
    if !is_sourced(wb, si, c) {
        return None;
    }
    wb.sheet(si)?.sources.get(&c).map(label)
}
