// Added by Socratics, 2026. See MODIFICATIONS.md.
//! The Source Inspector (the Sources toggle in the sheet-tab bar, on by default): a cell source
//! marker, a tiny square (`Tokens::sourced_mark`) in the top-left corner of a hard-coded number
//! that has a source link, with a tooltip naming the source; and for an active cell holding a
//! linked number, a tint over the cell (`Tokens::source_selected_fill`) and a source square beside
//! it whose click goes to the host. Unlinked numbers, text and formulas show nothing. Normal view
//! only: Page Layout view shows nothing. The markers are indicators only; the square is what is
//! clicked.

use egui::{Painter, Pos2, Rect, Stroke, pos2, vec2};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::{Sheet, SourceLink, Workbook};
use gridcraft_engine::source_state::{is_sourced, sourced_in};
use gridcraft_engine::{DocState, Selection};

use crate::SheetApp;
use crate::grid::Geo;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

/// Side of the painted square at zoom `z` (the size of the note indicator).
pub fn marker_size(z: f32) -> f32 {
    6.0 * z.max(0.7)
}

/// Smallest side of the source square, in points.
pub const SQUARE_MIN: f32 = 14.0;
/// Gap between the active cell (or its validation arrow) and the source square, clear of the
/// selection outline.
const SQUARE_GAP: f32 = 3.0;

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

/// Paints the markers of one pane (rows × cols). States are read from the cells; nothing is traced.
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
/// number. It gets the tint and the source square.
pub fn active_linked(app: &SheetApp, geo: &Geo, si: usize, sh: &Sheet, sel: &Selection) -> Option<CellRef> {
    if !app.ui.source_inspector || geo.paper.is_some() || app.editor.is_some() {
        return None;
    }
    let d = app.session.active()?;
    let anchor = sh.merge_at(sel.active).map_or(sel.active, |m| m.start);
    is_sourced(&d.wb, si, anchor).then_some(anchor)
}

/// The source square beside an active linked cell, in logical coordinates: `rect` is where it is
/// painted and `clip` the part of the cells it may show in (the panes of its row and those right of
/// its column, so it never covers the headers or the frozen columns of a scrolled cell).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceSquare {
    pub cell: CellRef,
    pub rect: Rect,
    pub clip: Rect,
}

impl SourceSquare {
    /// Where it is painted, and the cell it belongs to.
    pub fn new(geo: &Geo, sh: &Sheet, cell: CellRef) -> Option<SourceSquare> {
        if geo.hidden_under_frozen(sh, cell) || sh.row_height(cell.row) <= 0.0 {
            return None;
        }
        let area = cell_area(geo, sh, cell)?;
        if !area.is_positive() || !area.is_finite() {
            return None;
        }
        let side = (sh.row_height(cell.row) * geo.z).max(SQUARE_MIN);
        let split = pos2(geo.cells.left() + geo.frozen_w, geo.cells.top() + geo.frozen_h);
        let (x0, x1) = if cell.col < geo.fc { (geo.cells.left(), geo.cells.right()) } else { (split.x, geo.cells.right()) };
        let (y0, y1) = if cell.row < geo.fr { (geo.cells.top(), split.y) } else { (split.y, geo.cells.bottom()) };
        let clip = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        // Vertically centred on the cell's first row (its own row unless it anchors a merge).
        let mid = area.top() + sh.row_height(cell.row) * geo.z / 2.0;
        // Right of the cell, past its list arrow when it has one.
        let left = crate::grid::validation_arrow(sh, geo, cell).map_or(area.right(), |r| r.right()) + SQUARE_GAP;
        let right = Rect::from_center_size(pos2(left + side / 2.0, mid), vec2(side, side));
        // Wholly past the edge of the pane (the cell runs off the right of the window): beside its
        // left edge instead.
        let rect = if right.left() >= clip.right() {
            Rect::from_center_size(pos2(area.left() - SQUARE_GAP - side / 2.0, mid), vec2(side, side))
        } else {
            right
        };
        let square = SourceSquare { cell, rect, clip };
        (square.visible().is_positive() && square.visible().is_finite()).then_some(square)
    }

    /// The part of the square that shows.
    pub fn visible(&self) -> Rect {
        self.rect.intersect(self.clip)
    }

    /// Whether `p` (logical coordinates) is on the part of the square that shows.
    pub fn contains(&self, p: Pos2) -> bool {
        self.visible().contains(p)
    }

    /// Paints the square into the pane `p` paints, outlined in the marker blue, in the hover
    /// colour while `hot`.
    pub fn paint(&self, p: &Painter, hot: bool, t: &Tokens) {
        let p = p.with_clip_rect(p.clip_rect().intersect(self.clip));
        let side = self.rect.width();
        let fill = if hot { t.source_square_hover } else { t.source_square_fill };
        p.rect(self.rect, (side * 0.2).clamp(2.0, 4.0), fill, Stroke::new(1.0, t.sourced_mark), egui::StrokeKind::Inside);
        icons::paint(&p, self.rect.shrink(side * 0.2), Icon::ArrowUpRight, t.source_square_ink);
    }
}

/// `Heritage Trust Bank Statement September 2019 · Beginning Balance`: the document's name
/// without folder or extension, and the last part of the locator.
pub fn label(link: &SourceLink) -> String {
    let name = link.file.rsplit(['/', '\\']).next().unwrap_or(&link.file);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).replace('_', " ");
    let place = link.locator.rsplit(" / ").next().unwrap_or(&link.locator).trim();
    if place.is_empty() { stem } else { format!("{stem} · {place}") }
}

/// The tooltip of the marker (or source square) of `c`: its source. `None` for a cell without a
/// marker (an unlinked number, text, a formula).
pub fn tip(wb: &Workbook, si: usize, c: CellRef) -> Option<String> {
    if !is_sourced(wb, si, c) {
        return None;
    }
    wb.sheet(si)?.sources.get(&c).map(label)
}
