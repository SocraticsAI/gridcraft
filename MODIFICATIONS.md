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
   by document so a load of many cells from one file names it once: per sheet, one `<file path>` per
   distinct document holding a `<link ref locator>` per cell (an attribute it doesn't know, such as an
   older `<file>`'s `hash` and `size` or an older link's `storeRow`, is ignored; an unshipped flat version
   1 part, one element per cell naming its file, isn't read and is kept as unmodelled custom XML). New
   commands `source.set`, `source.get`
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
   gains Clear Sources, and the marker, tint, square and switch colours are theme tokens. Files: `crates/model`
   (`src/{cellmap,features,sheet}.rs`), `crates/engine` (`src/lib.rs`, the new `src/source_state.rs`,
   `src/cmd/{data,edit,format,insert,mod,sheet}.rs`, the new `src/cmd/source.rs`,
   `tests/command_sequence_fuzz.rs`), `crates/xlsx` (`src/{lib,passthrough,read,write}.rs`, the new
   `src/sources.rs`, `src/tests/passthrough_tests.rs`) and `crates/ui-egui`
   (`src/{control,grid,icons,lib,ribbon,tabs,theme,widgets}.rs`, the new `src/source_marks.rs` and
   `tests/source_links.rs`). Not proposed upstream.

4. **Read-only viewer** (2026). A host can show a workbook that looks exactly like one but can't be
   changed, for example a source document beside the user's own. `Session` gains `read_only` (off by
   default): a command runs, and if it left a workbook dirty (`DocState::is_dirty`, the engine's own
   test for "changed the file") it is rolled back and refused with the new `EngineError::ReadOnly`
   ("`cell.set` is not available: this workbook is read-only"; `EngineError::is_read_only_message`
   recognises it as text). View state is saved with the file but is not an edit, so selecting,
   scrolling, switching sheets, zooming, Page Layout view, finding and copying still run. Classifying
   by effect rather than by a list (or by the undoable flag, which counts `view.zoom` as an edit and
   `edit.undo` as not) covers new commands the day they land. Also refused: a command that leaves a
   new cut on the clipboard or arms the format painter or a Draw tool, and `file.save` / `file.saveAs`.
   A refused command's dialogs and edit requests are dropped; a read-only session keeps no undo
   history and a document's `revision` doesn't move. In the UI, `SheetApp::set_read_only(on)` (and
   `read_only()`) turns it on for an app: no in-cell, formula-bar or text-box edit starts (typing, F2,
   double-click), Enter moves instead of pasting a marked copy, paste does nothing, refused edits show no message, no fill,
   move, column/row resize, AutoFit, freeze, margin or object drag starts and the fill handle and list
   arrow aren't drawn, filter and list menus don't open, only `VIEWER_DIALOGS` open (Find without its
   Replace tab, Go To, Go To Special, Zoom, Statistics, Accessibility, Evaluate Formula, Open Link,
   Open Password), the cell and header menus offer Copy only, sheet tabs switch sheets but the New
   Sheet button, renaming, dragging and the tab menu are gone, and the formula bar has no Insert
   Function button. Turning it on drops an edit in progress, open menus and drags, a cut, the format
   painter and a Draw tool. `Chrome` gains `ribbon`, on by default, to hide the ribbon and its key
   tips (a viewer usually turns it off); the formula bar, sheet tabs and status bar stay. Files:
   `crates/engine` (`src/lib.rs`, `tests/command_sequence_fuzz.rs`, the new `tests/read_only.rs`) and
   `crates/ui-egui` (`src/{dialogs,formula_bar,grid,lib,tabs,text_box}.rs`, the new
   `tests/read_only_viewer.rs`). Not proposed upstream.

An earlier change, the host chrome switch, was merged upstream as
[storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877) and is in GridCraft from v0.10.0.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
