use super::{reduced_of, scheme_of};
use smudge_core::theme::ColorScheme;

#[test]
fn a_scheme_of_one_is_dark_and_an_unknown_value_is_light() {
    assert_eq!(scheme_of(0), ColorScheme::Light);
    assert_eq!(scheme_of(1), ColorScheme::Dark);
    assert_eq!(scheme_of(2), ColorScheme::Light);
    assert_eq!(
        scheme_of(3),
        ColorScheme::Light,
        "an unknown value is no preference"
    );
}

#[test]
fn a_motion_value_of_one_reduces_motion_and_an_unknown_value_does_not() {
    assert!(!reduced_of(0));
    assert!(reduced_of(1));
    assert!(!reduced_of(2), "an unknown value is no preference");
}
