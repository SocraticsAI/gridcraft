// Added by Socratics, 2026. See MODIFICATIONS.md.
//! A host can give GridCraft its own product name: the default comment author, the warning for a
//! decrypted workbook and the files it saves all carry it (its own test binary: the name is process-wide).

use std::io::Read as _;

use gridcraft_engine::{Session, core::brand, io};
use serde_json::json;

#[test]
fn the_hosts_name_reaches_comment_authors_notices_and_saved_files() {
    assert!(brand::set_name("Socratics"));

    let mut s = Session::new();
    assert_eq!(s.prefs.user_name, "Socratics User", "a new comment's default author");
    assert_eq!(io::decrypted_notice(), "This workbook was password-protected. Socratics saves it without the password.");

    s.new_workbook();
    let b64 = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap()["base64"].as_str().unwrap().to_string();
    let bytes = io::base64_decode(&b64).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut app = String::new();
    zip.by_name("docProps/app.xml").unwrap().read_to_string(&mut app).unwrap();
    assert!(app.contains("<Application>Socratics</Application>"), "{app}");
    assert!(!app.contains("GridCraft"), "{app}");
}
