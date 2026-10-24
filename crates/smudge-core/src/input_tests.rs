//! Tests for the platform-independent modifier behavior.

use super::Modifiers;

#[test]
fn command_follows_the_platform_modifier() {
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    let meta = Modifiers {
        meta: true,
        ..Modifiers::default()
    };
    let none = Modifiers::default();

    #[cfg(target_os = "macos")]
    {
        assert!(meta.command());
        assert!(!control.command());
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert!(control.command());
        assert!(!meta.command());
    }
    assert!(!none.command());
}
