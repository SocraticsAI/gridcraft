// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links (`gridcraft_model::SourceLink`) in the `.xlsx`, as a custom XML part, so a
//! workbook carries them through save and reopen (Excel keeps custom XML parts across its saves).
//!
//! The part is `customXml/itemN.xml` (the first `N` no other part uses), related from the workbook
//! by a `customXml` relationship, with an `itemPropsN.xml` naming its schema:
//! `<sources xmlns="urn:socratics:sources:1"><sheet name="Sheet1"><link ref="B2" file="…"
//! locator="…" storeRow="…"/></sheet></sources>`. Characters XML can't carry are written as
//! Excel's `_xHHHH_` escapes, so every link reopens exactly. Reading takes it into the model, and the
//! pass-through of unmodelled custom XML (`passthrough.rs`) leaves it out, so a save writes it
//! once, afresh.

use std::fmt::Write as _;
use std::sync::Arc;

use gridcraft_core::CellRef;
use gridcraft_model::{SourceLink, Workbook};

use crate::package::{Rel, normalize};
use crate::read::Ctx;
use crate::write::{Out, Rels, XML_DECL};
use crate::xml::{self, decode_xstring, encode_xstring, esc_attr};

/// Namespace of the part, and the schema its properties name.
pub const NS: &str = "urn:socratics:sources:1";
/// Content type of the item properties part.
const CT_PROPS: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
/// Namespace of the item properties part.
const NS_DS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";
/// Data store item id of the part (any GUID; the same in every file).
const ITEM_ID: &str = "{6F1C5E2A-3B7D-4C8E-9A0F-5D2B1E4C7A90}";
/// Most links read from one file.
const MAX_LINKS: usize = 2_000_000;

/// Writes the links of every sheet, if any sheet has one.
pub fn write(wb: &Workbook, out: &mut Out, wb_rels: &mut Rels) {
    if wb.sheets.iter().all(|s| s.sources.is_empty()) {
        return;
    }
    let mut x = format!("{XML_DECL}<sources xmlns=\"{NS}\">");
    for sh in wb.sheets.iter().filter(|s| !s.sources.is_empty()) {
        // The name the sheet is saved under (`write_xlsx` keeps the first 31 characters).
        let name: String = sh.name.chars().take(31).collect();
        let _ = write!(x, "<sheet name=\"{}\">", esc_attr(&name));
        for (c, l) in &sh.sources {
            let _ = write!(
                x,
                "<link ref=\"{}\" file=\"{}\" locator=\"{}\" storeRow=\"{}\"/>",
                c.a1(),
                attr(&l.file),
                attr(&l.locator),
                attr(&l.store_row)
            );
        }
        x.push_str("</sheet>");
    }
    x.push_str("</sources>");
    // Kept custom XML (written first) keeps its names: take the first free pair.
    let free = |out: &Out, name: &str| !out.has_part(name) && !out.kept.is_taken(name);
    let Some(n) = (1u32..=65_535).find(|n| free(out, &format!("customXml/item{n}.xml")) && free(out, &format!("customXml/itemProps{n}.xml"))) else {
        return;
    };
    let (item, props) = (format!("customXml/item{n}.xml"), format!("customXml/itemProps{n}.xml"));
    out.kept.reserve(&item);
    out.kept.reserve(&props);
    out.part(&item, None, x.into_bytes());
    let props_xml = format!(
        "{XML_DECL}<ds:datastoreItem ds:itemID=\"{ITEM_ID}\" xmlns:ds=\"{NS_DS}\"><ds:schemaRefs><ds:schemaRef ds:uri=\"{NS}\"/></ds:schemaRefs></ds:datastoreItem>"
    );
    out.part(&props, Some(CT_PROPS), props_xml.into_bytes());
    let mut rels = Rels::default();
    rels.add("customXmlProps", &format!("itemProps{n}.xml"));
    out.part(&format!("customXml/_rels/item{n}.xml.rels"), None, rels.xml().into_bytes());
    wb_rels.add("customXml", &format!("../customXml/item{n}.xml"));
}

/// A link field as an attribute value. Characters XML can't carry are written as Excel's
/// `_xHHHH_` escapes (and literal look-alikes protected), so the field reopens exactly.
fn attr(s: &str) -> String {
    esc_attr(&encode_xstring(s))
}

fn mentions_ns(bytes: &[u8]) -> bool {
    bytes.windows(NS.len()).any(|w| w == NS.as_bytes())
}

/// Whether `part` (the target of a workbook `customXml` relationship) is a source-links part
/// that [`read`] took into the model. Doesn't read the part again.
pub fn is_sources_part(cx: &Ctx<'_>, part: &str) -> bool {
    cx.sources_parts.contains(&normalize(part))
}

/// Reads the links of every source-links part related from the workbook into its sheets, by
/// sheet name. Unknown sheets, bad references and links without a file or locator are skipped.
/// Each part is read once, under the package's own per-part and total limits, and recorded in
/// `cx.sources_parts`.
pub fn read(cx: &mut Ctx<'_>, wb_rels: &[Rel], wb: &mut Workbook) {
    let mut count = 0usize;
    for r in wb_rels.iter().filter(|r| !r.external && r.kind.eq_ignore_ascii_case("customXml")) {
        if is_sources_part(cx, &r.target) {
            continue;
        }
        let bytes = match cx.pkg.read(&r.target) {
            Ok(Some(b)) => b,
            Ok(None) => continue,
            Err(e) => {
                cx.warn(format!("custom XML part {} could not be read for cell source links ({e})", r.target));
                continue;
            }
        };
        if !mentions_ns(&bytes) {
            continue;
        }
        let root = match xml::parse(&bytes) {
            Ok(root) if root.name == "sources" => root,
            Ok(_) => continue,
            Err(e) => {
                cx.warn(format!("cell source links could not be read ({e})"));
                continue;
            }
        };
        cx.sources_parts.push(normalize(&r.target));
        for se in root.kids("sheet") {
            let Some(si) = se.attr("name").and_then(|n| wb.sheet_index(n)) else { continue };
            let Some(sheet) = wb.sheets.get_mut(si) else { continue };
            let sheet = Arc::make_mut(sheet);
            for l in se.kids("link") {
                if count >= MAX_LINKS {
                    cx.warn(format!("only the first {MAX_LINKS} cell source links were read"));
                    return;
                }
                let Some(cell) = l.attr("ref").and_then(CellRef::parse).filter(CellRef::is_valid) else { continue };
                let (Some(file), Some(locator)) = (l.attr("file").map(decode_xstring), l.attr("locator").map(decode_xstring)) else { continue };
                if file.trim().is_empty() || locator.trim().is_empty() {
                    continue;
                }
                let store_row = l.attr("storeRow").map(decode_xstring).unwrap_or_default();
                sheet.sources.insert(cell, SourceLink { file, locator, store_row });
                count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;

    use gridcraft_core::CellRef;
    use gridcraft_model::{Sheet, SourceLink, Workbook};

    use crate::{read_xlsx, write_xlsx};

    fn link(locator: &str) -> SourceLink {
        SourceLink {
            file: "Data Room/2nd Tier Requests/Cash/Heritage_Trust_Bank_Statement_September_2019.pdf".into(),
            locator: locator.into(),
            store_row: format!("Heritage__statement!{locator}"),
        }
    }

    /// Parts whose content is a source-links part (not its properties part).
    fn sources_parts(bytes: &[u8]) -> Vec<String> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut out = vec![];
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut s = String::new();
            if f.read_to_string(&mut s).is_ok() && s.contains("<sources xmlns=\"urn:socratics:sources:1\"") {
                out.push(f.name().to_string());
            }
        }
        out
    }

    #[test]
    fn source_links_survive_an_xlsx_round_trip() {
        let mut wb = Workbook::new();
        wb.sheets.push(Arc::new(Sheet::new("Q3 Model")));
        let s0 = wb.sheet_mut(0).unwrap();
        s0.set_value(CellRef::new(1, 1), 1_318_609.0.into());
        s0.sources.insert(CellRef::new(1, 1), link("ACCOUNT SUMMARY / Beginning Balance"));
        s0.sources.insert(
            CellRef::new(2, 1),
            SourceLink { file: "Data Room/a & b <c>.xlsx".into(), locator: "GL Detail!C16102".into(), store_row: "x\"y".into() },
        );
        wb.sheet_mut(1).unwrap().sources.insert(CellRef::new(0, 0), link("Other"));
        let bytes = write_xlsx(&wb).unwrap();
        assert_eq!(sources_parts(&bytes), vec!["customXml/item1.xml".to_string()]);
        let (back, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(back.sheets[0].sources, wb.sheets[0].sources);
        assert_eq!(back.sheets[1].sources, wb.sheets[1].sources);
        // Saving the reopened file writes the part once: it isn't also kept as unmodelled custom XML.
        let again = write_xlsx(&back).unwrap();
        assert_eq!(sources_parts(&again).len(), 1);
        let (third, _) = read_xlsx(&again).unwrap();
        assert_eq!(third.sheets[0].sources, wb.sheets[0].sources);
    }

    #[test]
    fn control_characters_and_escape_lookalikes_in_links_reopen_exactly() {
        let mut wb = Workbook::new();
        let s0 = wb.sheet_mut(0).unwrap();
        s0.sources.insert(
            CellRef::new(0, 0),
            SourceLink { file: "Data Room/f.pdf".into(), locator: "Page 2\u{0C}Beginning Balance".into(), store_row: "S!A1".into() },
        );
        s0.sources.insert(
            CellRef::new(1, 0),
            SourceLink { file: "Data Room/a\u{01}b.pdf".into(), locator: "x\u{01}y\u{0B}z\u{7F}".into(), store_row: "r\u{1F}\u{FFFE}".into() },
        );
        // Text that already looks like Excel's `_xHHHH_` escape stays literal.
        s0.sources.insert(
            CellRef::new(2, 0),
            SourceLink { file: "Data Room/_x0041_.pdf".into(), locator: "_x005F_ and _x000C_".into(), store_row: String::new() },
        );
        let bytes = write_xlsx(&wb).unwrap();
        let (back, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(back.sheets[0].sources, wb.sheets[0].sources);
        let (third, _) = read_xlsx(&write_xlsx(&back).unwrap()).unwrap();
        assert_eq!(third.sheets[0].sources, wb.sheets[0].sources);
    }

    #[test]
    fn links_on_sheets_with_quotes_ampersands_and_accents_reopen_exactly() {
        let names = ["Q3 \"Café\" & Co's", "Q3 '19", "Résumé <Σ>"];
        let mut wb = Workbook::new();
        for n in names {
            wb.sheets.push(Arc::new(Sheet::new(n)));
        }
        for (i, n) in names.iter().enumerate() {
            let sh = wb.sheet_mut(i + 1).unwrap();
            sh.set_value(CellRef::new(1, 1), 1.0.into());
            sh.sources.insert(CellRef::new(1, 1), link(&format!("{n} / Beginning Balance")));
        }
        let bytes = write_xlsx(&wb).unwrap();
        let (back, report) = read_xlsx(&bytes).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        for (i, n) in names.iter().enumerate() {
            assert_eq!(back.sheets[i + 1].name, *n);
            assert_eq!(back.sheets[i + 1].sources, wb.sheets[i + 1].sources, "{n}");
        }
        let (third, _) = read_xlsx(&write_xlsx(&back).unwrap()).unwrap();
        for i in 1..=names.len() {
            assert_eq!(third.sheets[i].sources, wb.sheets[i].sources);
        }
    }

    #[test]
    fn a_links_part_over_64_mib_reopens_with_every_link_and_is_written_once() {
        let mut wb = Workbook::new();
        let s0 = wb.sheet_mut(0).unwrap();
        let long = "x".repeat(70_000);
        for i in 0..1_000u32 {
            s0.sources.insert(
                CellRef::new(i, 0),
                SourceLink { file: format!("Data Room/f{i}.pdf"), locator: format!("{i}:{long}"), store_row: format!("S!A{i}") },
            );
        }
        let bytes = write_xlsx(&wb).unwrap();
        let (back, report) = read_xlsx(&bytes).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(back.sheets[0].sources.len(), 1_000);
        assert_eq!(back.sheets[0].sources, wb.sheets[0].sources);
        let again = write_xlsx(&back).unwrap();
        assert_eq!(sources_parts(&again).len(), 1);
        let (third, _) = read_xlsx(&again).unwrap();
        assert_eq!(third.sheets[0].sources, wb.sheets[0].sources);
    }

    #[test]
    fn a_workbook_without_links_writes_no_sources_part() {
        let bytes = write_xlsx(&Workbook::new()).unwrap();
        assert!(sources_parts(&bytes).is_empty());
    }

    #[test]
    fn damaged_source_parts_are_skipped_without_failing_the_file() {
        let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
        let hostile = r#"<?xml version="1.0"?><sources xmlns="urn:socratics:sources:1"><sheet name="Nope"><link ref="A1" file="f" locator="l"/></sheet><sheet name="Data"><link ref="ZZZZ0" file="f" locator="l"/><link ref="B2" locator="no file"/><link ref="C3" file=" " locator="blank file"/><link ref="D4" file="ok.pdf" locator="kept"/></sheet></sources>"#;
        let (wb, _) = read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", hostile)], rel, "")).unwrap();
        let s = &wb.sheets[0];
        assert_eq!(s.sources.len(), 1);
        assert_eq!(s.sources.get(&CellRef::new(3, 3)).map(|l| l.locator.as_str()), Some("kept"));
        assert_eq!(s.sources.get(&CellRef::new(3, 3)).map(|l| l.store_row.as_str()), Some(""));
        let broken = "<sources xmlns=\"urn:socratics:sources:1\"><sheet";
        assert!(read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", broken)], rel, "")).is_ok());
    }
}
