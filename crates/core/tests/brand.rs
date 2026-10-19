//! The host's product name (its own test binary: the name is process-wide and set once).

use gridcraft_core::brand;

#[test]
fn the_host_sets_the_product_name_once() {
    assert_eq!(brand::name(), brand::DEFAULT_NAME, "GridCraft until a host says otherwise");
    assert!(!brand::set_name("   "), "a blank name is refused");
    assert_eq!(brand::name(), "GridCraft");
    assert!(brand::set_name(" Socratics "));
    assert_eq!(brand::name(), "Socratics", "trimmed");
    assert!(!brand::set_name("Other"), "only the first name takes effect");
    assert_eq!(brand::name(), "Socratics");
}
