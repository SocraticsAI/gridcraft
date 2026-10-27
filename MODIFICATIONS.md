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
3. **Cell source links** (2026). A sheet keeps a source link per cell (`Sheet::sources`: the file a
   hard-coded value came from and where in it), set by the host with the new commands `source.set`,
   `source.get` and `source.clear`, and Clear Sources in the ribbon (`edit.clearSources`). Links move with
   their cells (inserted and deleted rows, columns and cells, Move Columns, Sort, Remove Duplicates, cut and
   paste); copy and paste copies them; typing, filling or replacing over a cell, clearing its contents or
   merging it into a neighbour drops its link. They are saved in the `.xlsx` as a custom XML part (namespace
   `urn:socratics:sources:2`), grouped by file: per sheet, one `<file path>` per file holding a
   `<link ref locator>` per cell. Unknown attributes are ignored. A link's `file` read from a workbook is
   **untrusted**: any workbook can name an absolute path, a UNC share or a `..` escape, and the engine neither
   validates nor rejects one. A host that opens or fetches what a link names must resolve it against its own
   root and refuse anything that leaves it. Source links are also part of what sheet protection holds still:
   `source.set`, `source.clear` and Clear Sources are refused on a locked cell of a protected sheet, as typing
   in it is, while the row and column edits protection allows still carry links along. A linked hard-coded number carries a small
   blue marker in its top-left corner, with a tooltip naming its source, and is tinted while it is the active
   cell; nothing else shows a marker. A plain click on it raises `UiRequest::SourceClick` (sheet, cell and its
   rectangle on screen), passed to the host's `Services::source_click`. A Sources switch in the sheet-tab bar,
   at the start of the horizontal scrollbar's share, runs the UI command `view.sourceInspector` (on by
   default, saved in `UiState`) and turns markers, tint, tooltip and click off. The tab viewport is sized to
   its tabs so New Sheet follows the last tab — but never below what the active tab needs, so a long sheet
   name takes those pixels back instead of being clipped — and New Sheet is named for assistive tech. The marker, tint
   and switch colours are theme tokens. Files: `crates/model` (`src/{cellmap,features,sheet}.rs`),
   `crates/engine` (`src/lib.rs`, the new `src/source_state.rs`, `src/cmd/{data,edit,format,insert,mod,sheet}.rs`,
   the new `src/cmd/source.rs`, `tests/command_sequence_fuzz.rs`), `crates/xlsx`
   (`src/{lib,passthrough,read,write}.rs`, the new `src/sources.rs`, `src/tests/passthrough_tests.rs`) and
   `crates/ui-egui` (`src/{control,grid,lib,ribbon,tabs,theme,widgets}.rs`, `tests/sheet_tabs.rs`, the new
   `src/source_marks.rs` and `tests/source_links.rs`). Not proposed upstream.

An earlier change, the host chrome switch, was merged upstream as
[storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877) and is in GridCraft from v0.10.0.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
