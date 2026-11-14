//! The color scheme the desktop holds, watched over the settings portal.
//!
//! Modern desktops keep the user's light or dark preference as the
//! `color-scheme` value of `org.freedesktop.appearance`, which the settings
//! portal serves over the session bus. The watcher reads it once when the
//! app opens and again whenever the portal announces a change, and hands
//! each scheme to the window through the run loop's user events. Where
//! there is no bus to ask, for example on a headless display, the watcher
//! never starts and the app keeps the theme it set itself.

use std::thread;

use smudge_core::theme::ColorScheme;

mod dbus;
#[cfg(test)]
#[path = "color_scheme/dbus_tests.rs"]
mod tests;

/// The namespace that holds the color scheme.
const APPEARANCE: &str = "org.freedesktop.appearance";

/// The value of the namespace that names the user's preference.
const COLOR_SCHEME: &str = "color-scheme";

/// The match rule that forwards the portal's settings changes, naming the
/// portal as sender so no other connection's signals get through.
const MATCH_RULE: &str = "type='signal',interface='org.freedesktop.portal.Settings',\
     member='SettingChanged',sender='org.freedesktop.portal.Desktop'";

/// Starts the watcher that reports the desktop's color scheme through
/// `deliver`, on a thread of its own.
///
/// The thread ends when the bus goes away or the process does, and a
/// desktop that serves no portal simply never delivers anything, so a
/// failure here only costs the following.
pub(crate) fn start(deliver: impl Fn(ColorScheme) + Send + 'static) {
    let spawned = thread::Builder::new()
        .name(String::from("smudge-color-scheme"))
        .spawn(move || watch(deliver));
    if let Err(error) = spawned {
        eprintln!("smudge: the color scheme watcher could not start: {error}");
    }
}

/// Connects to the bus, reads the scheme once and reports every change,
/// until the connection breaks.
fn watch(deliver: impl Fn(ColorScheme)) {
    let _ = run(deliver);
    // The bus went away, or the desktop serves no portal. The app keeps the
    // scheme it holds, which is the honest answer on a machine with none.
}

fn run(deliver: impl Fn(ColorScheme)) -> Option<()> {
    let mut connection = dbus::Connection::connect()?;
    connection.hello()?;
    connection.add_match(MATCH_RULE)?;
    // Signals are trusted by their sender, which the bus fills in; the
    // name to expect is the portal's current owner, when it has one. A
    // portal that is not running yet is woken by the read below, and its
    // first signal settles the name then.
    connection.expect_signals_from(dbus::PORTAL);
    if let Some(value) = connection.read_setting(APPEARANCE, COLOR_SCHEME) {
        deliver(scheme_of(value));
    }
    loop {
        let (namespace, key, value) = connection.wait_for_changed()?;
        if namespace == APPEARANCE && key == COLOR_SCHEME {
            deliver(scheme_of(value));
        }
    }
}

/// The scheme a portal value names. A value of one is a dark preference;
/// anything else, including an unknown value, is no preference, which the
/// specification says to treat as zero, and which paints light here.
fn scheme_of(value: u32) -> ColorScheme {
    if value == 1 {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
