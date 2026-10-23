# Modifications by Socratics

This branch (`socratics`) of the [SocraticsAI/gridcraft](https://github.com/SocraticsAI/gridcraft) fork is
GridCraft as released upstream ([storytold/gridcraft](https://github.com/storytold/gridcraft), Apache-2.0 OR
MIT), plus the changes below. Socratics builds its spreadsheet from this branch. Every file the changes touch
carries a notice on its first line.

## Changes

1. **Product name set by the host** (2026). A new `gridcraft_core::brand` module holds the product name shown
   and saved, "GridCraft" by default. An embedding host can set it once at startup. Window and dialog titles,
   the default note author, `docProps/app.xml`'s `<Application>`, pivot `refreshedBy`, the PDF creator and
   producer, and several messages read it instead of a fixed "GridCraft". Files: `crates/core` (the new
   `src/brand.rs`, `src/lib.rs`, `tests/brand.rs`), `crates/engine` (`src/cmd/{extra,file,solver,spelling,whatif}.rs`,
   `src/io.rs`, `src/lib.rs`, `src/tests.rs`, the new `tests/host_brand.rs`), `crates/pdf` (`Cargo.toml`,
   `src/lib.rs`), `crates/ui-egui` (`src/{backstage,dialogs,formula_bar,grid,lib,ribbon}.rs`) and `crates/xlsx`
   (`src/pivot.rs`, `src/write.rs`, `src/xls/formula.rs`, `src/xls/mod.rs`). Not proposed upstream.
2. **Automate switch for the host** (2026). `Chrome` gains an `automate` field, on by default. An embedding
   host can turn it off to hide the ribbon's Automate tab and refuse its dialogs (Macros, Command Palette,
   Agent Control, Action Journal, About), however they are reached: Alt+F8, the VBA notice's link,
   `ui.ribbonTab`. Files: `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/ribbon.rs`, the new
   `crates/ui-egui/tests/host_automate.rs`, and `crates/ui-egui/tests/host_chrome.rs` (its `Chrome` literals
   take the new field from the default). Not proposed upstream yet.
3. **Cell source links** (2026). A sheet keeps a source link per cell (`Sheet::sources`: the document a
   hard-coded value was read from, where in it, and the lineage store row). Links move with their
   cells (inserting and deleting rows, columns and cells, Move Columns, Sort, Remove Duplicates,
   cut and paste, dragging a selection), copy and paste copies them, and typing, filling or
   replacing over a cell, clearing its contents and merging it into a neighbour drop its link. They are
   saved in the `.xlsx` as a custom XML part (namespace `urn:socratics:sources:1`) and read back. New
   commands `source.set`, `source.get`, `source.clear` and `source.trace` (a formula traced through its
   references, across sheets, 3-D references, names and tables, to linked values). Only hard-coded
   values carry a marker in their top-left corner: a linked value is shown as sourced, an unlinked number
   as unsourced, with a tooltip naming the source or saying there is none; formulas (and spilled values)
   are traced by `source.trace` but show no marker. The markers are indicators only (a click on one
   just selects the cell). When the active cell (only the active cell, even in a range) holds a linked
   value, it is tinted and a source square with an up-right arrow floats just right of it (left of it
   when that would be wholly off the pane; past a list-validation arrow; under the fill handle, which
   wins). Hovering the square names the source; clicking it raises `UiRequest::SourceClick` for the
   active cell, which goes to the host's `Services::source_click`, and leaves the selection as it is.
   View ▸ Sources ▸ Source Inspector (the UI command `view.sourceInspector`, `{on?: bool}`, on by
   default and saved with the other UI preferences in `UiState`) turns markers, tint, square and
   tooltips off; the links stay in the model. Normal view only. The ribbon also gains Clear Sources,
   and the marker, tint and square colours are theme tokens. Files: `crates/model`
   (`src/{cellmap,features,sheet,store}.rs`), `crates/engine` (`src/lib.rs`, the new `src/source_state.rs`,
   `src/cmd/{data,edit,format,formulas,insert,mod,sheet}.rs`, the new `src/cmd/source.rs`,
   `tests/command_sequence_fuzz.rs`), `crates/xlsx` (`src/{lib,passthrough,read,write}.rs`, the new
   `src/sources.rs`, `src/tests/passthrough_tests.rs`) and `crates/ui-egui`
   (`src/{control,grid,icons,lib,ribbon,theme,widgets}.rs`, the new `src/source_marks.rs` and
   `tests/source_links.rs`). Not proposed upstream.

An earlier change, the host chrome switch, was merged upstream as
[storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877) and is in GridCraft from v0.10.0.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
