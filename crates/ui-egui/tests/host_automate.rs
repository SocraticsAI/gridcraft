// Added by Socratics, 2026. See MODIFICATIONS.md.
//! A host that doesn't offer GridCraft's automation can turn off the Automate tab and its dialogs.

use gridcraft_engine::Session;
use gridcraft_ui_egui::{Chrome, Services, SheetApp};
use serde_json::json;

const AUTOMATE_DIALOGS: [&str; 5] = ["macros", "commandSearch", "agents", "journal", "about"];

fn app(chrome: Chrome) -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Services::default());
    app.chrome = chrome;
    app
}

#[test]
fn the_automate_tab_and_its_dialogs_are_on_by_default() {
    let mut app = app(Chrome::default());
    app.run_or_alert("ui.ribbonTab", json!({"tab": "Automate"}));
    assert_eq!(app.ui.ribbon_tab, "Automate");
    for name in AUTOMATE_DIALOGS {
        app.dialog = None;
        app.open_dialog(name, json!({}));
        assert!(app.dialog.is_some(), "{name} opens");
    }
}

#[test]
fn without_automate_the_tab_cannot_be_selected() {
    let mut app = app(Chrome { automate: false, ..Chrome::default() });
    app.run_or_alert("ui.ribbonTab", json!({"tab": "Automate"}));
    assert_eq!(app.ui.ribbon_tab, "Home", "selecting Automate lands on Home");
    app.run_or_alert("ui.ribbonTab", json!({"tab": "Data"}));
    assert_eq!(app.ui.ribbon_tab, "Data", "other tabs still select");
}

#[test]
fn without_automate_its_dialogs_never_open() {
    let mut app = app(Chrome { automate: false, ..Chrome::default() });
    for name in AUTOMATE_DIALOGS {
        app.open_dialog(name, json!({}));
        assert!(app.dialog.is_none(), "{name} stays closed");
    }
    app.open_dialog("goTo", json!({}));
    assert!(app.dialog.is_some(), "other dialogs still open");
}
