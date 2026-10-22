// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source markers: a small triangle in the top-left corner of a cell whose value is traced to
//! source documents (`Tokens::sourced_mark`) or isn't (`Tokens::unsourced_mark`), its tooltip, and
//! where a click on it lands. Normal view only: Page Layout view shows no markers.

use egui::{Painter, Pos2, Rect, Stroke, pos2, vec2};
use gridcraft_engine::DocState;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::{Sheet, SourceLink, Workbook};
use gridcraft_engine::source_state::{SourceState, source_state, source_states_in, trace};

use crate::grid::Geo;
use crate::theme::Tokens;

/// Side of the painted triangle at zoom `z` (the size of the note indicator).
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

/// Where a click or hover counts as on the marker of `c` (at least 8 points square), in logical
/// (left-to-right) coordinates.
pub fn marker_rect(geo: &Geo, sh: &Sheet, c: CellRef) -> Option<Rect> {
    let r = cell_area(geo, sh, c)?;
    let s = marker_size(geo.z).max(8.0);
    Some(Rect::from_min_size(r.left_top(), vec2(s, s)))
}

/// Paints the markers of one pane (rows × cols). States come from the engine's cache; nothing is
/// traced here.
#[allow(clippy::too_many_arguments)]
pub fn paint_quadrant(d: &DocState, p: &Painter, geo: &Geo, si: usize, sh: &Sheet, rows: (u32, u32), cols: (u32, u32), t: &Tokens) {
    if geo.paper.is_some() || rows.0 > rows.1 || cols.0 > cols.1 {
        return;
    }
    let range = RangeRef::new(CellRef::new(rows.0, cols.0), CellRef::new(rows.1, cols.1));
    let s = marker_size(geo.z);
    for (c, state) in source_states_in(d, si, range) {
        if sh.row_height(c.row) == 0.0 || sh.col_width(c.col) == 0.0 {
            continue;
        }
        let Some(r) = cell_area(geo, sh, c) else { continue };
        let color = match state {
            SourceState::Sourced => t.sourced_mark,
            SourceState::Unsourced => t.unsourced_mark,
        };
        p.add(egui::Shape::convex_polygon(vec![r.left_top(), pos2(r.left() + s, r.top()), pos2(r.left(), r.top() + s)], color, Stroke::NONE));
    }
}

/// The cell whose marker is under `p` (logical coordinates), if any. Normal view only.
pub fn hit(geo: &Geo, d: &DocState, si: usize, sh: &Sheet, p: Pos2) -> Option<CellRef> {
    if geo.paper.is_some() || !geo.in_cells(p) {
        return None;
    }
    let c = geo.cell_at(sh, p);
    let anchor = sh.merges.iter().find(|m| m.contains(c)).map_or(c, |m| m.start);
    source_state(d, si, anchor)?;
    marker_rect(geo, sh, anchor)?.contains(p).then_some(anchor)
}

/// `Heritage Trust Bank Statement September 2019 · Beginning Balance`: the document's name
/// without folder or extension, and the last part of the locator.
pub fn label(link: &SourceLink) -> String {
    let name = link.file.rsplit(['/', '\\']).next().unwrap_or(&link.file);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s).replace('_', " ");
    let place = link.locator.rsplit(" / ").next().unwrap_or(&link.locator).trim();
    if place.is_empty() { stem } else { format!("{stem} · {place}") }
}

/// The tooltip of the marker of `c`.
pub fn tip(wb: &Workbook, si: usize, c: CellRef) -> Option<String> {
    let t = trace(wb, si, c);
    let is_formula = wb.sheet(si).and_then(|sh| sh.cell(c)).is_some_and(|x| x.formula.is_some());
    if t.untraceable {
        return Some("Can't be traced to a source: it uses INDIRECT or OFFSET".into());
    }
    if !t.unsourced.is_empty() {
        return Some(if !is_formula {
            "No source for this value".into()
        } else if t.unsourced.len() == 1 {
            "1 input has no source".into()
        } else {
            format!("{} inputs have no source", t.unsourced.len())
        });
    }
    match t.links.as_slice() {
        [] => None,
        [one] => Some(label(&one.link)),
        many => Some(format!("From {} sources", many.len())),
    }
}
