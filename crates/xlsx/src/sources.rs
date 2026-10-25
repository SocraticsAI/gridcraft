// Added by Socratics, 2026. See MODIFICATIONS.md.
//! Cell source links (`gridcraft_model::SourceLink`) in the `.xlsx`, as a custom XML part, so a
//! workbook carries them through save and reopen (Excel keeps custom XML parts across its saves).
//!
//! The part is `customXml/itemN.xml` (the first `N` no other part uses), related from the workbook
//! by a `customXml` relationship, with an `itemPropsN.xml` naming its schema. Links are grouped by
//! their document, so a load of many cells from one file names the file once:
//! `<sources xmlns="urn:socratics:sources:2"><sheet name="Sheet1"><file path="…" hash="…"
//! size="…"><link ref="B2" locator="…"/></file></sheet></sources>`: one `<file>` per distinct
//! path, fingerprint (`hash`) and size in the sheet, `hash` and `size` only when the links have
//! them. Characters XML can't carry are written as Excel's `_xHHHH_` escapes, so every link
//! reopens exactly. Attributes this version doesn't know (an older link's `storeRow`) are ignored.
//! Reading takes it into the model, and the pass-through of unmodelled custom XML
//! (`passthrough.rs`) leaves it out, so a save writes it once, afresh.
//!
//! Version 1 of the namespace wrote one flat `<link file=… locator=…/>` per cell. It never shipped
//! and isn't read: a version 1 part is unmodelled custom XML, kept as it is.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use gridcraft_core::CellRef;
use gridcraft_model::{SourceLink, Workbook};

use crate::package::{Rel, normalize};
use crate::read::Ctx;
use crate::write::{Out, Rels, XML_DECL};
use crate::xml::{self, decode_xstring, encode_xstring, esc_attr};

/// Namespace of the part, and the schema its properties name.
pub const NS: &str = "urn:socratics:sources:2";
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
        // One group per document: path, fingerprint and size. A malformed hash isn't written, so
        // its links group with the ones that have none.
        let mut groups: BTreeMap<(&str, Option<&str>, Option<u64>), Vec<(&CellRef, &SourceLink)>> = BTreeMap::new();
        for (c, l) in &sh.sources {
            let hash = l.hash.as_deref().filter(|h| SourceLink::is_hash(h));
            groups.entry((l.file.as_str(), hash, l.size)).or_default().push((c, l));
        }
        for ((file, hash, size), links) in groups {
            let _ = write!(x, "<file path=\"{}\"", attr(file));
            // A well-formed hash is plain hex: nothing to escape.
            if let Some(h) = hash {
                let _ = write!(x, " hash=\"{h}\"");
            }
            if let Some(n) = size {
                let _ = write!(x, " size=\"{n}\"");
            }
            x.push('>');
            for (c, l) in links {
                let _ = write!(x, "<link ref=\"{}\" locator=\"{}\"/>", c.a1(), attr(&l.locator));
            }
            x.push_str("</file>");
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
/// sheet name. Unknown sheets, files without a path, bad references and links without a locator
/// are skipped.
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
            for fe in se.kids("file") {
                let Some(file) = fe.attr("path").map(decode_xstring).filter(|f| !f.trim().is_empty()) else { continue };
                // A malformed fingerprint is dropped; the links stay.
                let hash = fe.attr("hash").filter(|h| SourceLink::is_hash(h)).map(str::to_string);
                let size = fe.attr("size").and_then(|n| n.parse::<u64>().ok());
                for l in fe.kids("link") {
                    if count >= MAX_LINKS {
                        cx.warn(format!("only the first {MAX_LINKS} cell source links were read"));
                        return;
                    }
                    let Some(cell) = l.attr("ref").and_then(CellRef::parse).filter(CellRef::is_valid) else { continue };
                    let Some(locator) = l.attr("locator").map(decode_xstring).filter(|s| !s.trim().is_empty()) else { continue };
                    sheet.sources.insert(cell, SourceLink { file: file.clone(), locator, hash: hash.clone(), size });
                    count += 1;
                }
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
            ..Default::default()
        }
    }

    /// Parts whose content is a source-links part (not its properties part).
    fn sources_parts(bytes: &[u8]) -> Vec<String> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut out = vec![];
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut s = String::new();
            if f.read_to_string(&mut s).is_ok() && s.contains("<sources xmlns=\"urn:socratics:sources:2\"") {
                out.push(f.name().to_string());
            }
        }
        out
    }

    fn part_text(bytes: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut s = String::new();
        z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    }

    fn part_compressed(bytes: &[u8], name: &str) -> u64 {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        z.by_name(name).unwrap().compressed_size()
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
            SourceLink { file: "Data Room/a & b <c>.xlsx".into(), locator: "GL Detail!C16102 \"x\"".into(), ..Default::default() },
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
            SourceLink { file: "Data Room/f.pdf".into(), locator: "Page 2\u{0C}Beginning Balance".into(), ..Default::default() },
        );
        s0.sources.insert(
            CellRef::new(1, 0),
            SourceLink { file: "Data Room/a\u{01}b.pdf".into(), locator: "x\u{01}y\u{0B}z\u{7F}\u{1F}\u{FFFE}".into(), ..Default::default() },
        );
        // Text that already looks like Excel's `_xHHHH_` escape stays literal.
        s0.sources.insert(
            CellRef::new(2, 0),
            SourceLink { file: "Data Room/_x0041_.pdf".into(), locator: "_x005F_ and _x000C_".into(), ..Default::default() },
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
            s0.sources
                .insert(CellRef::new(i, 0), SourceLink { file: format!("Data Room/f{i}.pdf"), locator: format!("{i}:{long}"), ..Default::default() });
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
    fn fingerprints_survive_an_xlsx_round_trip_and_absent_ones_stay_absent() {
        let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let mut wb = Workbook::new();
        let s0 = wb.sheet_mut(0).unwrap();
        s0.sources.insert(CellRef::new(0, 0), SourceLink { hash: Some(hash.into()), size: Some(u64::MAX), ..link("with") });
        s0.sources.insert(CellRef::new(1, 0), link("without"));
        s0.sources.insert(CellRef::new(2, 0), SourceLink { size: Some(0), ..link("size only") });
        let bytes = write_xlsx(&wb).unwrap();
        let (back, report) = read_xlsx(&bytes).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(back.sheets[0].sources, wb.sheets[0].sources);
        let without = back.sheets[0].sources.get(&CellRef::new(1, 0)).unwrap();
        assert_eq!((without.hash.as_deref(), without.size), (None, None));
        let (third, _) = read_xlsx(&write_xlsx(&back).unwrap()).unwrap();
        assert_eq!(third.sheets[0].sources, wb.sheets[0].sources);
    }

    #[test]
    fn malformed_fingerprints_in_a_file_are_dropped_and_the_links_kept() {
        let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
        let good = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let part = format!(
            r#"<?xml version="1.0"?><sources xmlns="urn:socratics:sources:2"><sheet name="Data"><file path="f" hash="{}" size="-4"><link ref="A1" locator="upper"/></file><file path="f" hash="abc" size="1e3"><link ref="A2" locator="short"/></file><file path="f" hash="{good}" size="42"><link ref="A3" locator="ok"/></file><file path="f" size="99999999999999999999999"><link ref="A4" locator="huge"/></file></sheet></sources>"#,
            good.to_uppercase()
        );
        let (wb, _) = read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", &part)], rel, "")).unwrap();
        let s = &wb.sheets[0];
        assert_eq!(s.sources.len(), 4);
        for r in [0, 1, 3] {
            let l = s.sources.get(&CellRef::new(r, 0)).unwrap();
            assert_eq!((l.hash.as_deref(), l.size), (None, None), "row {r}");
        }
        let ok = s.sources.get(&CellRef::new(2, 0)).unwrap();
        assert_eq!((ok.hash.as_deref(), ok.size), (Some(good), Some(42)));
    }

    #[test]
    fn a_legacy_store_row_attribute_is_ignored_and_not_written_back() {
        let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
        let part = r#"<?xml version="1.0"?><sources xmlns="urn:socratics:sources:2"><sheet name="Data"><file path="f.pdf"><link ref="A1" locator="l" storeRow="S!C12"/><link ref="A2" locator="m" storeRow=""/></file></sheet></sources>"#;
        let (wb, report) = read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", part)], rel, "")).unwrap();
        assert!(!report.warnings.iter().any(|w| w.contains("source")), "{:?}", report.warnings);
        let s = &wb.sheets[0];
        assert_eq!(s.sources.len(), 2);
        assert_eq!(s.sources.get(&CellRef::new(0, 0)), Some(&SourceLink { file: "f.pdf".into(), locator: "l".into(), ..Default::default() }));
        let saved = write_xlsx(&wb).unwrap();
        let parts = sources_parts(&saved);
        assert_eq!(parts.len(), 1);
        assert!(!part_text(&saved, &parts[0]).contains("storeRow"));
        let (back, _) = read_xlsx(&saved).unwrap();
        assert_eq!(back.sheets[0].sources, s.sources);
    }

    #[test]
    fn a_workbook_without_links_writes_no_sources_part() {
        let bytes = write_xlsx(&Workbook::new()).unwrap();
        assert!(sources_parts(&bytes).is_empty());
    }

    #[test]
    fn damaged_source_parts_are_skipped_without_failing_the_file() {
        let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
        let hostile = r#"<?xml version="1.0"?><sources xmlns="urn:socratics:sources:2"><sheet name="Nope"><file path="f"><link ref="A1" locator="l"/></file></sheet><sheet name="Data"><link ref="A9" locator="outside a file"/><file><link ref="B2" locator="no path"/></file><file path=" "><link ref="C3" locator="blank path"/></file><file path="ok.pdf"><link ref="ZZZZ0" locator="l"/><link ref="E5"/><link ref="E6" locator=" "/><link ref="D4" locator="kept"/><file path="nested"><link ref="F6" locator="n"/></file></file></sheet></sources>"#;
        let (wb, _) = read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", hostile)], rel, "")).unwrap();
        let s = &wb.sheets[0];
        assert_eq!(s.sources.len(), 1);
        assert_eq!(s.sources.get(&CellRef::new(3, 3)).map(|l| l.locator.as_str()), Some("kept"));
        let broken = "<sources xmlns=\"urn:socratics:sources:2\"><sheet";
        assert!(read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", broken)], rel, "")).is_ok());
    }

    #[test]
    fn links_are_saved_grouped_by_file_and_reopen_exactly() {
        let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let mut wb = Workbook::new();
        wb.sheets.push(Arc::new(Sheet::new("Q3 \"Café\" & <Co>")));
        let a = "Data Room/a & b <c> \"d\".pdf";
        let b = "Data Room/Café/_x0041_\u{01}.xlsx";
        let mk = |file: &str, locator: &str, hash: Option<&str>, size: Option<u64>| SourceLink {
            file: file.into(),
            locator: locator.into(),
            hash: hash.map(str::to_string),
            size,
        };
        let s0 = wb.sheet_mut(0).unwrap();
        s0.sources.insert(CellRef::new(0, 0), mk(a, "Page 1 / Total & \"net\"", Some(hash), Some(1234)));
        s0.sources.insert(CellRef::new(1, 0), mk(a, "Page 2 <x>", Some(hash), Some(1234)));
        // Same path, no fingerprint: its own group.
        s0.sources.insert(CellRef::new(2, 0), mk(a, "Page 3", None, None));
        s0.sources.insert(CellRef::new(3, 0), mk(b, "GL Detail!C16102\u{0C}", None, Some(0)));
        s0.sources.insert(CellRef::new(4, 0), mk(b, "GL Detail!C2\u{1F}", None, Some(0)));
        // A malformed fingerprint isn't written: it groups with the unfingerprinted ones.
        s0.sources.insert(CellRef::new(5, 0), mk(a, "Page 4", Some("ABC"), None));
        let s1 = wb.sheet_mut(1).unwrap();
        s1.sources.insert(CellRef::new(9, 3), mk(b, "x", Some(hash), None));
        s1.sources.insert(CellRef::new(0, 1), mk(a, "_x005F_", None, None));
        let bytes = write_xlsx(&wb).unwrap();
        let parts = sources_parts(&bytes);
        assert_eq!(parts.len(), 1);
        let text = part_text(&bytes, &parts[0]);
        // Sheet 1 has three groups (a with fingerprint, a without, b), sheet 2 has two.
        assert_eq!(text.matches("<file ").count(), 5, "{text}");
        assert_eq!(text.matches("<link ").count(), 8, "{text}");
        assert!(!text.contains(" file=\""), "{text}");
        let (back, report) = read_xlsx(&bytes).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let mut expected = wb.sheets[0].sources.clone();
        if let Some(l) = expected.get_mut(&CellRef::new(5, 0)) {
            l.hash = None;
        }
        assert_eq!(back.sheets[0].sources, expected);
        assert_eq!(back.sheets[1].sources, wb.sheets[1].sources);
        let (third, _) = read_xlsx(&write_xlsx(&back).unwrap()).unwrap();
        assert_eq!(third.sheets[0].sources, expected);
        assert_eq!(third.sheets[1].sources, wb.sheets[1].sources);
    }

    #[test]
    fn a_hundred_thousand_links_to_one_file_save_small_and_reopen_exactly() {
        let hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let file = "Data Room/2nd Tier Requests/Cash/Heritage_Trust_Bank_Statement_September_2019.xlsx";
        let mut wb = Workbook::new();
        let s0 = wb.sheet_mut(0).unwrap();
        for i in 0..100_000u32 {
            s0.sources.insert(
                CellRef::new(i / 10, i % 10),
                SourceLink { file: file.into(), locator: format!("Sheet1!R{i}"), hash: Some(hash.into()), size: Some(48_213_337) },
            );
        }
        let bytes = write_xlsx(&wb).unwrap();
        let parts = sources_parts(&bytes);
        assert_eq!(parts.len(), 1);
        let text = part_text(&bytes, &parts[0]);
        // The document is named once; what's left is each link's own cell and locator plus a
        // little markup (about 22 MB as one flat element per link naming its file).
        assert_eq!(text.matches(file).count(), 1);
        assert_eq!(text.matches(hash).count(), 1);
        let own: usize = wb.sheets[0].sources.iter().map(|(c, l)| c.a1().len() + l.locator.len()).sum();
        let markup = (text.len() - own) / 100_000;
        assert!(markup <= 40, "{markup} bytes of markup per link");
        assert!(text.len() < 6_000_000, "the part is {} bytes", text.len());
        assert!(part_compressed(&bytes, &parts[0]) < 1_000_000);
        let (back, report) = read_xlsx(&bytes).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(back.sheets[0].sources, wb.sheets[0].sources);
    }

    #[test]
    fn a_version_1_part_loads_no_links_and_is_kept_as_custom_xml() {
        let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
        let old = r#"<?xml version="1.0"?><sources xmlns="urn:socratics:sources:1"><sheet name="Data"><link ref="A1" file="f.pdf" locator="l"/></sheet></sources>"#;
        let (wb, _) = read_xlsx(&crate::tests::minimal("<sheetData/>", &[("customXml/item1.xml", old)], rel, "")).unwrap();
        assert!(wb.sheets[0].sources.is_empty());
        let saved = write_xlsx(&wb).unwrap();
        assert!(sources_parts(&saved).is_empty());
        let kept = String::from_utf8_lossy(&saved).contains("urn:socratics:sources:1")
            || (0..zip::ZipArchive::new(std::io::Cursor::new(&saved[..])).unwrap().len()).any(|i| {
                let mut z = zip::ZipArchive::new(std::io::Cursor::new(&saved[..])).unwrap();
                let mut t = String::new();
                z.by_index(i).unwrap().read_to_string(&mut t).is_ok() && t.contains("urn:socratics:sources:1")
            });
        assert!(kept, "the version 1 part is kept as unmodelled custom XML");
        let (back, _) = read_xlsx(&saved).unwrap();
        assert!(back.sheets[0].sources.is_empty());
    }
}
