// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links (`gridcraft_model::SourceLink`) in the `.xlsx`, as a custom XML part, so a
//! workbook carries them through save and reopen (Excel keeps custom XML parts across its saves).
//!
//! The part is `customXml/itemN.xml` (the first `N` no other part uses), related from the workbook
//! by a `customXml` relationship, with an `itemPropsN.xml` naming its schema:
//! `<sources xmlns="urn:socratics:sources:1"><sheet name="Sheet1"><link ref="B2" file="…"
//! locator="…" storeRow="…"/></sheet></sources>`. Reading takes it into the model, and the
//! pass-through of unmodelled custom XML (`passthrough.rs`) leaves it out, so a save writes it
//! once, afresh.

use std::fmt::Write as _;
use std::sync::Arc;

use gridcraft_core::CellRef;
use gridcraft_model::{SourceLink, Workbook};

use crate::package::Rel;
use crate::read::Ctx;
use crate::write::{Out, Rels, XML_DECL};
use crate::xml::{self, esc_attr};

/// Namespace of the part, and the schema its properties name.
pub const NS: &str = "urn:socratics:sources:1";
/// Content type of the item properties part.
const CT_PROPS: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
/// Namespace of the item properties part.
const NS_DS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";
/// Data store item id of the part (any GUID; the same in every file).
const ITEM_ID: &str = "{6F1C5E2A-3B7D-4C8E-9A0F-5D2B1E4C7A90}";
/// Largest part read.
const MAX_PART: u64 = 64 * 1024 * 1024;
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
                esc_attr(&l.file),
                esc_attr(&l.locator),
                esc_attr(&l.store_row)
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

fn mentions_ns(bytes: &[u8]) -> bool {
    bytes.windows(NS.len()).any(|w| w == NS.as_bytes())
}

/// Whether `part` (the target of a workbook `customXml` relationship) is a source-links part.
pub fn is_sources_part(cx: &mut Ctx<'_>, part: &str) -> bool {
    match cx.pkg.read_limited(part, MAX_PART) {
        Ok(Some(b)) => mentions_ns(&b) && xml::parse(&b).is_ok_and(|root| root.name == "sources"),
        _ => false,
    }
}

/// Reads the links of every source-links part related from the workbook into its sheets, by
/// sheet name. Unknown sheets, bad references and links without a file or locator are skipped.
pub fn read(cx: &mut Ctx<'_>, wb_rels: &[Rel], wb: &mut Workbook) {
    let mut count = 0usize;
    for r in wb_rels.iter().filter(|r| !r.external && r.kind.eq_ignore_ascii_case("customXml")) {
        let bytes = match cx.pkg.read_limited(&r.target, MAX_PART) {
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
                let (Some(file), Some(locator)) = (l.attr("file"), l.attr("locator")) else { continue };
                if file.trim().is_empty() || locator.trim().is_empty() {
                    continue;
                }
                let store_row = l.attr("storeRow").unwrap_or_default().to_string();
                sheet.sources.insert(cell, SourceLink { file: file.to_string(), locator: locator.to_string(), store_row });
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
