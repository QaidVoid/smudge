//! The color scheme and the reduced-motion preference the desktop holds,
//! watched over the settings portal.
//!
//! Modern desktops keep the user's light or dark preference as the
//! `color-scheme` value of `org.freedesktop.appearance`, and the wish for
//! less motion as its `reduced-motion` value, which the settings portal
//! serves over the session bus. A watcher reads one value once when it
//! starts and again whenever the portal announces a change, and hands each
//! one to the window through the run loop's user events. Where there is no
//! bus to ask, for example on a headless display, or a portal too old to
//! know the value, the watcher reports nothing and the app keeps what it
//! set itself.

use std::thread;

use smudge_core::theme::ColorScheme;

mod dbus;

/// The namespace that holds the appearance settings.
const APPEARANCE: &str = "org.freedesktop.appearance";

/// The value of the namespace that names the user's color preference.
const COLOR_SCHEME: &str = "color-scheme";

/// The value of the namespace that names the user's motion preference.
const REDUCED_MOTION: &str = "reduced-motion";

/// The match rule that forwards the portal's settings changes. It names the
/// portal as sender, and the bus forwards a broadcast that names a
/// well-known sender only from that name's current owner, so no other
/// connection's signals get through, even across a portal restart.
const MATCH_RULE: &str = "type='signal',interface='org.freedesktop.portal.Settings',\
     member='SettingChanged',sender='org.freedesktop.portal.Desktop'";

/// Starts the watcher that reports the desktop's color scheme through
/// `deliver`, on a thread of its own.
///
/// The thread ends when the bus goes away or the process does, and a
/// desktop that serves no portal simply never delivers anything, so a
/// failure here only costs the following.
pub(crate) fn start(deliver: impl Fn(ColorScheme) + Send + 'static) {
    spawn("smudge-color-scheme", COLOR_SCHEME, move |value| {
        deliver(scheme_of(value));
    });
}

/// Starts the watcher that reports whether the desktop asks for reduced
/// motion through `deliver`, on a thread of its own, as [`start`] does for
/// the color scheme.
pub(crate) fn start_reduced_motion(deliver: impl Fn(bool) + Send + 'static) {
    spawn("smudge-reduced-motion", REDUCED_MOTION, move |value| {
        deliver(reduced_of(value));
    });
}

/// Starts a thread named `name` that reports the appearance value `key`
/// through `deliver`.
fn spawn(name: &str, key: &'static str, deliver: impl Fn(u32) + Send + 'static) {
    let spawned = thread::Builder::new()
        .name(String::from(name))
        // The bus went away, or the desktop serves no portal. The app keeps
        // what it holds, which is the honest answer on a machine with none.
        .spawn(move || run(key, deliver));
    if let Err(error) = spawned {
        eprintln!("smudge: the {name} watcher could not start: {error}");
    }
}

/// Connects to the bus, reads the value `key` once and reports every
/// change, until the connection breaks.
fn run(key: &str, deliver: impl Fn(u32)) -> Option<()> {
    let mut connection = dbus::Connection::connect()?;
    connection.hello()?;
    connection.add_match(MATCH_RULE)?;
    if let Some(value) = connection.read_settled(APPEARANCE, key) {
        deliver(value);
    }
    loop {
        let (namespace, changed, value) = connection.wait_for_changed()?;
        if namespace == APPEARANCE && changed == key {
            deliver(value);
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

/// Whether a portal value asks for reduced motion. A value of one does;
/// anything else, including an unknown value, is no preference, which the
/// specification says to treat as zero.
fn reduced_of(value: u32) -> bool {
    value == 1
}

#[cfg(test)]
#[path = "color_scheme_tests.rs"]
mod tests;
