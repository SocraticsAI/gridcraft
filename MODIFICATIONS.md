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
   hard-coded value was read from and where in it). The host writes them; links move with their
   cells (inserting and deleting rows, columns and cells, Move Columns, Sort, Remove Duplicates,
   cut and paste, dragging a selection), copy and paste copies them, and typing, filling or
   replacing over a cell, clearing its contents and merging it into a neighbour drop its link. They are
   saved in the `.xlsx` as a custom XML part (namespace `urn:socratics:sources:2`) and read back, grouped
   by document so a load of many cells from one file names it once: per sheet, one `<file path hash size>`
   per distinct document holding a `<link ref locator>` per cell (an attribute it doesn't know, such as
   an older link's `storeRow`, is ignored; an unshipped flat version 1 part, one element per cell naming
   its file, isn't read and is kept as unmodelled custom XML). New commands `source.set`, `source.get`
   and `source.clear`. Only linked hard-coded numbers carry a marker, a small blue square in their
   top-left corner with a tooltip naming the source; unlinked numbers, linked text or logicals (labels,
   headers), formulas and spilled values show nothing. The markers are indicators only (a click on one
   just selects the cell). When the active cell (only the active cell, even in a range) holds a linked
   number, it is tinted and a source square with an up-right arrow floats just right of it (left of it
   when that would be wholly off the pane; past a list-validation arrow; under the fill handle, which
   wins). Hovering the square names the source; clicking it raises `UiRequest::SourceClick` for the
   active cell, which goes to the host's `Services::source_click`, and leaves the selection as it is.
   A Sources toggle in the sheet-tab bar, at the start of the horizontal scrollbar's share, runs the UI
   command `view.sourceInspector` (`{on?: bool}`, on by default and saved with the other UI preferences in
   `UiState`) and turns markers, tint, square and tooltips off; the links stay in the model. It is a
   small labelled switch (a 32×16 pill with a white thumb, Socratics' small toggle), dropping its label
   when the bar is too short to keep the scrollbar usable. The tab viewport is sized to its tabs, so New
   Sheet follows the last tab; New Sheet is named for assistive tech. Normal view only. The ribbon also
   gains Clear Sources, and the marker, tint, square and switch colours are theme tokens. A link may also
   carry its document's fingerprint, `hash` (SHA-256, 64 lowercase hex digits) and `size` (bytes), so the
   host can find the document after it moves; `source.set` takes them (refusing malformed ones), and they
   are saved in JSON and as `hash`/`size` attributes of the link's `<file>` in the `.xlsx` part (a malformed one is dropped on
   load, keeping the link). `source.list` returns every link in the workbook (sheet order, then row, then
   column). `source.setMissing` (`{missing: [{sheet, cell}]}`) replaces the document's set of cells whose
   source the host could not find: display state kept on the open document, never saved, journaled,
   undone or marking it dirty. A linked number in it draws the same treatment in grey (marker, tint, and a
   square holding a "?" with the tooltip "Source missing"); its square's click still goes to the host.
   Files: `crates/model`
   (`src/{cellmap,features,sheet}.rs`), `crates/engine` (`src/lib.rs`, the new `src/source_state.rs`,
   `src/cmd/{data,edit,format,insert,mod,sheet}.rs`, the new `src/cmd/source.rs`,
   `tests/command_sequence_fuzz.rs`), `crates/xlsx` (`src/{lib,passthrough,read,write}.rs`, the new
   `src/sources.rs`, `src/tests/passthrough_tests.rs`) and `crates/ui-egui`
   (`src/{control,grid,icons,lib,ribbon,tabs,theme,widgets}.rs`, the new `src/source_marks.rs` and
   `tests/source_links.rs`). Not proposed upstream.

An earlier change, the host chrome switch, was merged upstream as
[storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877) and is in GridCraft from v0.10.0.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
