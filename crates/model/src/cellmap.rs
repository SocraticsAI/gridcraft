// Modified by Socratics, 2026: cell source links. See MODIFICATIONS.md.
//! Serde helpers for maps keyed by `CellRef`. JSON object keys must be strings, so the keys are
//! written as `A1`-style text.

use std::collections::BTreeMap;

use gridcraft_core::CellRef;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer, V: Serialize>(map: &BTreeMap<CellRef, V>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(map.iter().map(|(k, v)| (k.a1(), v)))
}

pub fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(d: D) -> Result<BTreeMap<CellRef, V>, D::Error> {
    let raw = BTreeMap::<String, V>::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let cell = CellRef::parse(&k).ok_or_else(|| D::Error::custom(format!("invalid cell reference `{k}`")))?;
        out.insert(cell, v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::{Comment, Hyperlink, Sheet, SourceLink};
    use gridcraft_core::CellRef;

    #[test]
    fn comments_and_hyperlinks_survive_json() {
        let mut s = Sheet::new("S");
        let a1 = CellRef::new(0, 0);
        let c3 = CellRef::new(2, 2);
        s.comments.insert(
            a1,
            Comment {
                author: "Example".into(),
                text: "note".into(),
                replies: vec![("B".into(), "r".into())],
                threaded: true,
                resolved: false,
                visible: false,
                dates: vec![],
            },
        );
        s.hyperlinks.insert(c3, Hyperlink { target: "Sheet1!A1".into(), tooltip: Some("jump".into()), external: false });
        let json = serde_json::to_string(&s).expect("serialize");
        let back: Sheet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.comments, s.comments);
        assert_eq!(back.hyperlinks, s.hyperlinks);
    }

    #[test]
    fn source_links_survive_json_and_stay_out_of_it_when_empty() {
        let mut s = Sheet::new("S");
        let empty = serde_json::to_string(&s).expect("serialize");
        assert!(!empty.contains("\"sources\""), "an unlinked sheet writes no sources key: {empty}");
        s.sources.insert(
            CellRef::new(1, 1),
            SourceLink {
                file: "Data Room/2nd Tier Requests/Cash/Heritage_Trust_Bank_Statement_September_2019.pdf".into(),
                locator: "ACCOUNT SUMMARY / Beginning Balance".into(),
                store_row: "Heritage_Trust_Bank_Statement_September_2019__statement!C12".into(),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("\"B2\""), "keys are A1 text: {json}");
        assert!(json.contains("\"storeRow\""), "fields are camelCase: {json}");
        assert!(!json.contains("\"hash\"") && !json.contains("\"size\""), "an unfingerprinted link writes neither: {json}");
        let back: Sheet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.sources, s.sources);
        assert!(back.sources.values().all(|l| l.hash.is_none() && l.size.is_none()));
        // JSON written before links existed still loads.
        let mut value = serde_json::to_value(&s).expect("to value");
        value.as_object_mut().expect("object").remove("sources");
        let old: Sheet = serde_json::from_value(value).expect("older JSON loads");
        assert!(old.sources.is_empty());
    }

    #[test]
    fn source_fingerprints_survive_json_and_bad_ones_are_dropped() {
        let hash = "0123456789abcdef".repeat(4);
        let mut s = Sheet::new("S");
        let link = SourceLink {
            file: "Data Room/a.pdf".into(),
            locator: "p1".into(),
            store_row: String::new(),
            hash: Some(hash.clone()),
            size: Some(1_234_567),
        };
        s.sources.insert(CellRef::new(0, 0), link);
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains(&format!("\"hash\":\"{hash}\"")) && json.contains("\"size\":1234567"), "{json}");
        let back: Sheet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.sources, s.sources);
        // A malformed fingerprint is dropped, the link and the rest of the file still load.
        for (bad_hash, bad_size) in [
            (serde_json::json!("ABC"), serde_json::json!(-1)),
            (serde_json::json!(hash.to_uppercase()), serde_json::json!(1.5)),
            (serde_json::json!(7), serde_json::json!("12")),
            (serde_json::json!({"x": 1}), serde_json::json!(null)),
        ] {
            let mut value = serde_json::to_value(&s).expect("to value");
            value["sources"]["A1"]["hash"] = bad_hash;
            value["sources"]["A1"]["size"] = bad_size;
            let loaded: Sheet = serde_json::from_value(value).expect("loads");
            let l = loaded.sources.get(&CellRef::new(0, 0)).expect("link kept");
            assert_eq!((l.hash.as_deref(), l.size, l.locator.as_str()), (None, None, "p1"));
        }
        assert!(SourceLink::is_hash(&hash));
        assert!(!SourceLink::is_hash(&hash[1..]) && !SourceLink::is_hash(&format!("{hash}0")) && !SourceLink::is_hash(&"g".repeat(64)));
    }
}
