# Modifications by Socratics

This branch (`socratics`) of the [SocraticsAI/gridcraft](https://github.com/SocraticsAI/gridcraft) fork is
GridCraft as released upstream ([storytold/gridcraft](https://github.com/storytold/gridcraft), Apache-2.0 OR
MIT), plus the changes below. Socratics builds its spreadsheet from this branch. Every file the changes touch
carries a notice on its first line.

## Changes

1. **Host chrome switch** (2026). `SheetApp` gains a non-persisted `chrome` field, so an embedding host can
   hide the title bar, and the File tab with the Backstage view it opens. Both stay on by default. Files:
   `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/ribbon.rs`, and the new `crates/ui-egui/tests/host_chrome.rs`.
   Merged upstream as [storytold/gridcraft#1877](https://github.com/storytold/gridcraft/pull/1877); this entry
   and its notices go when the branch is rebased onto a release that includes it.
2. **Product name set by the host** (2026). A new `gridcraft_core::brand` module holds the product name shown
   and saved, "GridCraft" by default. An embedding host can set it once at startup. Window and dialog titles,
   the default note author, `docProps/app.xml`'s `<Application>`, pivot `refreshedBy`, the PDF creator and
   producer, and several messages read it instead of a fixed "GridCraft". Files: `crates/core` (the new
   `src/brand.rs`, `src/lib.rs`, `tests/brand.rs`), `crates/engine` (`src/cmd/{extra,file,solver,spelling,whatif}.rs`,
   `src/io.rs`, `src/lib.rs`, `src/tests.rs`, the new `tests/host_brand.rs`), `crates/pdf` (`Cargo.toml`,
   `src/lib.rs`), `crates/ui-egui` (`src/{backstage,dialogs,formula_bar,grid,lib,ribbon}.rs`) and `crates/xlsx`
   (`src/pivot.rs`, `src/write.rs`, `src/xls/formula.rs`, `src/xls/mod.rs`). Not proposed upstream.

The licences, `NOTICE` and `ATTRIBUTION.md` are upstream's, unchanged.
