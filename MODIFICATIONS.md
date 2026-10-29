# Modifications by Socratics

Each `socratics-vX.Y.Z` tag of the [SocraticsAI/gridcraft](https://github.com/SocraticsAI/gridcraft) fork is
GridCraft vX.Y.Z as released upstream ([storytold/gridcraft](https://github.com/storytold/gridcraft), Apache-2.0
OR MIT), plus the changes below. Socratics builds its spreadsheet from these tags. Every file the changes touch
carries a notice on its first line.

## Changes

1. **Product name set by the host** (2026). A new `gridcraft_core::brand` module holds the product name shown
   and saved, "GridCraft" by default. An embedding host can set it once at startup. Window and dialog titles,
   the default note author, `docProps/app.xml`'s `<Application>`, pivot `refreshedBy`, the PDF creator and
   producer, the File › Account product name, the comment on the hidden names GridCraft keeps its data in,
   and several messages read it instead of a fixed "GridCraft". Files: `crates/core` (the new
   `src/brand.rs`, `src/lib.rs`, `tests/brand.rs`), `crates/engine` (`src/cmd/{extra,file,formulas,spelling,stash}.rs`,
   `src/cmd/solver/reports.rs`, `src/cmd/whatif/scenarios.rs`, `src/io.rs`, `src/lib.rs`, `src/tests.rs`, the
   new `tests/host_brand.rs`), `crates/pdf` (`Cargo.toml`, `src/lib.rs`), `crates/ui-egui`
   (`src/{backstage,dialogs,formula_bar}.rs`, `src/app/{commands,editing,files,prefs}.rs`,
   `src/backstage/options.rs`, `src/dialogs/{about,macros,show}.rs`, `src/grid/{interact,menus}.rs`,
   `src/ribbon/automate.rs`) and `crates/xlsx` (`src/pivot/write.rs`, `src/write.rs`, `src/xls.rs`,
   `src/xls/formula.rs`). The new `gridcraft-pdf` →
   `gridcraft-core` dependency is two crates inside layer L0, so `xtask/src/layers.rs` lists it in
   `INTRA_LAYER_ORDER` the way `core` → `numfmt` already was; without that entry `cargo xtask layers`
   (and so `cargo xtask ci`) fails. Not proposed upstream.
2. **Automate switch for the host** (2026). `Chrome` gains an `automate` field, on by default. An embedding
   host can turn it off to hide the ribbon's Automate tab and refuse its dialogs (Macros, Command Palette,
   Agent Control, Action Journal, About), however they are reached: Alt+F8, the VBA notice's link,
   `ui.ribbonTab`. Files: `crates/ui-egui` (`src/{lib,ribbon,services}.rs`, `src/app/{commands,files}.rs`,
   the new `tests/host_automate.rs`, and `tests/it/host_chrome.rs`, whose `Chrome` literals take the new
   field from the default) and `apps/gridcraft-web/src/web.rs` (its `?chrome=none` literal does the same, for
   this field and `sources`). Not proposed upstream yet.
3. **Cell source links** (2026). A sheet keeps a source link per cell (`Sheet::sources`: the file a
   hard-coded value came from and where in it), set by the host with the new commands `source.set`,
   `source.get`, `source.clear` and `source.clearAll` (every sheet, one undo step), and Clear Sources in the
   ribbon (`edit.clearSources`). Only values take links: `source.set` refuses a formula cell, a cell inside
   a spill range (a dynamic-array or legacy array formula's result, e.g. `A2` under `=SEQUENCE(3)` in
   `A1`), and a `file` that isn't a plain relative path inside the folder (`SourceLink::relative_path`: no
   absolute path, drive letter, UNC share or backslash, URL scheme, `.`/`..` or empty segment, or control
   character). Links move
   with their cells (inserted and deleted rows, columns and cells, Move Columns, Sort, Remove Duplicates, cut
   and paste); copy and paste within the workbook copies them, a paste into another workbook doesn't; typing,
   filling or replacing over a cell, clearing its contents or merging it into a neighbour drops its link. The
   map is shared copy-on-write between a workbook and its undo snapshots (`Arc`), and links to one file share
   its name, so an edit that leaves links alone costs nothing however many there are. They are saved in the
   `.xlsx` as a custom XML part (namespace `urn:socratics:sources:2`), grouped by file: per sheet, one
   `<file path>` per file holding a `<link ref locator>` per cell; unknown attributes are ignored. Reading
   drops (and counts in a warning) links whose file isn't a relative path, links on a formula cell or a cell
   inside an array formula's range in the file (what `source.set` refuses), and links past 2,000,000.
   `gridcraft_xlsx::WriteOptions { omit_sources }` (`io::save_bytes_with`) saves an `.xlsx` or `.json` copy
   without links, which name internal folders and files and which Excel keeps on its own saves. A host that
   opens what a link names must still resolve it against its own root and refuse anything that leaves it.
   Source links are part of what sheet protection holds still: setting or clearing one on a locked cell of a
   protected sheet is refused, while the row and column edits protection allows still carry links along.
   On screen, only when the host sets the new `Chrome::sources` (off by default, so the tab bar is then
   upstream's): a linked hard-coded number carries a small blue marker in its top-left corner, with a
   tooltip naming its source, and is tinted while it is the active cell; a plain click on it raises
   `UiRequest::SourceClick` (sheet, cell and its rectangle), passed to the host's `Services::source_click`; a
   Sources switch in the sheet-tab bar, at the start of the horizontal scrollbar's share, runs the UI command
   `view.sourceInspector` (on by default, saved in `UiState`) and turns markers, tint, tooltip and click off.
   The switch sits just before the handle that resizes the tabs' share, so it never takes room from the tabs,
   and New Sheet is named for assistive tech. Marker, tint and switch colours are theme tokens; the new strings are translated.
   **API changes** for a host taking this fork: `Sheet::sources` (`Arc<SourceLinks>`; change it through
   `Sheet::sources_mut` / `remove_sources_in`), `SourceLink::file` (`Arc<str>`), `Chrome::sources`,
   `UiRequest::SourceClick` (a new variant: an exhaustive `match` on `UiRequest` needs an arm),
   `Services::source_click`, `Ctx::sources_moved`, `UiState::source_inspector` and `GridState::source_tip` are new public items or fields, so struct literals built outside their constructors need
   them. Files: `crates/model` (`src/{lib,sheet}.rs`, the new `src/source_link.rs`), `crates/engine`
   (`src/{cmd,io,lib}.rs`, the new `src/source_state.rs`, `src/cmd/{edit,format,sheet}.rs`,
   `src/cmd/data/{sort,tools}.rs`, `src/cmd/edit/{clipboard,paste_special}.rs`, `src/cmd/insert/table.rs`, the
   new `src/cmd/source.rs`, `tests/it/command_sequence_fuzz.rs`, which checks every link stays on the value it
   was set on), `crates/xlsx` (`src/{lib,read,sheet_read,write}.rs`, `src/passthrough/{read,write}.rs`, the new
   `src/sources.rs`, `src/tests/passthrough_tests.rs`) and `crates/ui-egui` (`src/{app,control,grid,lib,paper_view,services,tabs,theme,widgets}.rs`,
   `src/app/commands.rs`, `src/grid/{interact,quadrant,show}.rs`, `src/ribbon/home.rs`, `src/i18n/*.rs`
   (`ja.rs` is the master list), the new `src/source_marks.rs` and `tests/source_links.rs`). Not
   proposed upstream.
4. **Streamed rows and the XML element limit** (2026). GridCraft v0.13.0 refuses a part whose tree would hold
   more than 1,000,000 elements (`MAX_XML_NODES`), but counted every element of the rows it streams out of the
   tree too, so a worksheet of more than about 500,000 cells read as damaged and lost every row after the
   limit. A streamed element now takes its elements out of the count; one row that large is still refused.
   Files: `crates/xlsx/src/xml.rs`. Meant to be proposed upstream.

An earlier change, the host chrome switch, was merged upstream as
[storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877) and is in GridCraft from v0.10.0.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
