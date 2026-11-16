//! Wire tests for the hand-rolled D-Bus client.
//!
//! The codec checks answer the byte layout and its alignments; the round
//! trip drives the real client against a bus that speaks the protocol the
//! way the session bus does, through one socket pair. The fake side reads
//! and writes every message apart from the client's own encoder, headers
//! and bodies alike, so a mistake shared by both cannot pass the round
//! trip.

use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use super::{Body, Connection, Cursor, candidate_path, session_addresses};
use crate::color_scheme::MATCH_RULE;

/// Opens the socket pair a test's client and fake bus speak over, with a
/// read timeout on both ends so a regression fails instead of hanging.
fn socket_pair() -> (UnixStream, UnixStream) {
    let (client_side, bus_side) = UnixStream::pair().expect("a socket pair opens");
    for side in [&client_side, &bus_side] {
        side.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("the socket takes a read timeout");
    }
    (client_side, bus_side)
}

#[test]
fn a_body_holds_its_strings_and_a_variant_reads_back() {
    let mut body = Body::default();
    body.string("org.freedesktop.appearance");
    body.string("color-scheme");
    let mut cursor = Cursor::over(&body.0, false);
    assert_eq!(
        cursor.string().as_deref(),
        Some("org.freedesktop.appearance")
    );
    assert_eq!(cursor.string().as_deref(), Some("color-scheme"));
    assert_eq!(cursor.string(), None, "the body holds exactly two strings");
}

#[test]
fn a_variant_of_another_type_is_refused() {
    // A variant that names a string, built the way the wire lays it out:
    // the signature, then the value with the string's four-byte alignment.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x01s\0");
    bytes.extend_from_slice(&5_u32.to_le_bytes());
    bytes.extend_from_slice(b"light\0");
    assert_eq!(Cursor::over(&bytes, false).variant_u32(), None);
}

#[test]
fn a_variant_u32_reads_through_strings_that_misalign_it() {
    let mut body = Body::default();
    body.string("org.freedesktop.appearance");
    body.string("color-scheme");
    // The variant starts right after the strings, and its integer value
    // keeps the four-byte alignment relative to the body's start.
    let mut bytes = body.0;
    bytes.extend_from_slice(b"\x01u\0");
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    let mut cursor = Cursor::over(&bytes, false);
    cursor.string();
    cursor.string();
    assert_eq!(cursor.variant_u32(), Some(2));
}

#[test]
fn an_address_path_decodes_its_escapes_even_to_non_text_bytes() {
    let path = candidate_path("unix:path=/tmp/session-%ff/bus,guid=abc")
        .expect("the candidate names a path");
    assert_eq!(
        std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()),
        b"/tmp/session-\xff/bus",
        "the decoded path keeps the byte the escape named"
    );
    // The properties may sit in any order, and only `path=` answers.
    assert_eq!(
        candidate_path("unix:guid=abc,path=/run/user/1000/bus"),
        Some(std::path::PathBuf::from("/run/user/1000/bus"))
    );
    assert_eq!(
        candidate_path("unix:abstract=/tmp/garbage"),
        None,
        "a transport without a path answers nothing"
    );
}

#[test]
fn a_session_address_without_a_path_falls_back_to_the_runtime_directory() {
    let runtime_dir = std::ffi::OsStr::new("/run/user/1000");
    let fallback = vec![std::path::PathBuf::from("/run/user/1000/bus")];
    assert_eq!(
        session_addresses(
            Some("unix:abstract=/tmp/dbus-x,guid=abc;unix:tmpdir=/tmp"),
            Some(runtime_dir)
        ),
        fallback,
        "an address with no unix path still finds the well-known socket"
    );
    assert_eq!(session_addresses(None, Some(runtime_dir)), fallback);
    assert_eq!(
        session_addresses(
            Some("unix:abstract=/tmp/dbus-x;unix:path=/tmp/bus"),
            Some(runtime_dir)
        ),
        vec![std::path::PathBuf::from("/tmp/bus")],
        "a path the address names comes before the well-known socket"
    );
}

#[test]
fn a_client_exchanges_the_portal_round_trip_with_a_bus() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        assert_eq!(hello.member, "Hello", "the conversation opens with Hello");
        assert_eq!(
            hello.path.as_deref(),
            Some("/org/freedesktop/DBus"),
            "Hello names the bus's own object"
        );
        assert_eq!(
            hello.destination.as_deref(),
            Some("org.freedesktop.DBus"),
            "Hello goes to the bus itself"
        );
        assert_eq!(
            hello.body_signature, None,
            "Hello carries no arguments, so it names no signature"
        );
        bus.reply_string(hello.serial, ":1.42");

        let add_match = bus.read_call();
        assert_eq!(add_match.member, "AddMatch");
        assert_eq!(
            add_match.body_signature.as_deref(),
            Some("s"),
            "the rule travels as one string"
        );
        assert_eq!(
            add_match.body_strings(),
            vec![String::from(MATCH_RULE)],
            "the rule travels as the watcher wrote it"
        );
        assert!(
            MATCH_RULE.contains("sender='org.freedesktop.portal.Desktop'"),
            "only the rule's sender keeps other connections' signals out"
        );
        bus.reply_empty(add_match.serial);

        let read_one = bus.read_call();
        assert_eq!(read_one.member, "ReadOne");
        assert_eq!(
            read_one.interface.as_deref(),
            Some("org.freedesktop.portal.Settings"),
            "the call names the settings interface"
        );
        assert_eq!(
            read_one.path.as_deref(),
            Some("/org/freedesktop/portal/desktop")
        );
        assert_eq!(
            read_one.body_signature.as_deref(),
            Some("ss"),
            "a namespace and a key travel with the call"
        );
        assert_eq!(
            read_one.body_strings(),
            vec![
                String::from("org.freedesktop.appearance"),
                String::from("color-scheme"),
            ]
        );
        bus.reply_variant(false, read_one.serial, 2);

        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match(MATCH_RULE)
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the portal answers the scheme it holds"
    );
    assert_eq!(
        connection.wait_for_changed(),
        Some((
            String::from("org.freedesktop.appearance"),
            String::from("color-scheme"),
            1
        )),
        "the change the bus forwards arrives"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_reply_that_answers_another_call_is_skipped() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        // Other connections may write to this one: a return and an error
        // that answer no call the client has outstanding.
        bus.reply_string(999, ":1.99");
        bus.reply_error(998);
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection
        .hello()
        .expect("only a reply to the outstanding serial answers the call");
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_return_that_answers_another_call_does_not_answer_this_one() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // A return that answers another serial, carrying a value of its
        // own; then the answer this call waits for.
        bus.reply_variant(false, 999, 7);
        let read_one = bus.read_call();
        bus.reply_variant(false, read_one.serial, 2);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "a return for another serial does not answer this call"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_change_seen_during_a_failed_read_is_the_start() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // The portal announces dark, and then answers neither read.
        let read_one = bus.read_call();
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
        bus.reply_error(read_one.serial);
        let read = bus.read_call();
        bus.reply_error(read.serial);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(1),
        "the change the failed read waited past is the value to start from"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_newer_change_than_the_reply_is_read_again() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // A portal that replaced the one the read reached announces dark
        // before the old one answers with light; the read that follows
        // reaches the new portal.
        let read_one = bus.read_call();
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
        bus.reply_variant(false, read_one.serial, 0);
        let again = bus.read_call();
        bus.reply_variant(false, again.serial, 1);
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 0);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(1),
        "the value the current portal holds is the start"
    );
    assert_eq!(
        connection.wait_for_changed(),
        Some((
            String::from("org.freedesktop.appearance"),
            String::from("color-scheme"),
            0
        )),
        "a change after the read still arrives"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn an_older_change_than_the_reply_is_read_again() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // A portal on its way out announced dark; its replacement answers
        // the read with the light set since, and announces nothing.
        let read_one = bus.read_call();
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
        bus.reply_variant(false, read_one.serial, 2);
        let again = bus.read_call();
        bus.reply_variant(false, again.serial, 2);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the stale change does not override the current portal's value"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_failed_read_again_starts_from_the_change_that_prompted_it() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // A replacing portal announces dark before the old one answers
        // light, and then answers neither read that follows.
        let read_one = bus.read_call();
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
        bus.reply_variant(false, read_one.serial, 0);
        let again = bus.read_call();
        bus.reply_error(again.serial);
        let read = bus.read_call();
        bus.reply_error(read.serial);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(1),
        "the change, not the reply it raced, is the start"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn reads_that_all_race_a_change_start_from_the_last_change() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // Every read races a replacing portal's change to dark, which
        // arrives before the old portal's light.
        for _ in 0..3 {
            let read_one = bus.read_call();
            bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
            bus.reply_variant(false, read_one.serial, 0);
        }
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(1),
        "the last change, not the reply it raced, is the start"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn changes_of_other_settings_leave_the_read_settings_change() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // The portal announces dark and then many other settings, and
        // answers neither read.
        let read_one = bus.read_call();
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
        for index in 0..40 {
            bus.signal_changed("org.example.other", &format!("key-{index}"), 7);
        }
        bus.reply_error(read_one.serial);
        let read = bus.read_call();
        bus.reply_error(read.serial);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_settled("org.freedesktop.appearance", "color-scheme"),
        Some(1),
        "the other settings pushed the read setting's change out"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_change_addressed_to_the_client_is_skipped() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        // Another connection sends the client a change by name; only the
        // portal's broadcast that follows is the portal's.
        bus.signal_changed_to_client(1);
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 0);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    assert_eq!(
        connection.wait_for_changed(),
        Some((
            String::from("org.freedesktop.appearance"),
            String::from("color-scheme"),
            0
        )),
        "the addressed change was taken for the portal's"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_version_one_portal_answers_the_older_read() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        let read_one = bus.read_call();
        assert_eq!(
            read_one.member, "ReadOne",
            "the client asks the newer method first"
        );
        // A version-one portal names no such method.
        bus.reply_error(read_one.serial);
        let read = bus.read_call();
        assert_eq!(read.member, "Read", "the client falls back to Read");
        bus.reply_variant_in_variant(false, read.serial, 2);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the older portal's wrapped answer still names the scheme"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_reply_serial_after_a_signature_field_is_still_found() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // The signature field comes first; a parser that loses its place
        // there misses the serial and takes the fallback answer instead.
        let read_one = bus.read_call();
        bus.reply_variant_signature_first(false, read_one.serial, 2);
        bus.reply_variant(false, read_one.serial, 7);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the serial is read wherever the fields sit"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_reply_serial_after_a_signature_field_is_found_in_either_order() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string_big_endian(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty_big_endian(add_match.serial);
        let read_one = bus.read_call();
        bus.reply_variant_signature_first(true, read_one.serial, 2);
        bus.reply_variant(true, read_one.serial, 7);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the big-endian reply is read by its own order"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn an_unknown_header_field_is_skipped_by_its_type() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // Fields this client has no name for lead the block; a parser
        // that stops at them misses the serial and takes the fallback
        // answer instead.
        let read_one = bus.read_call();
        bus.reply_variant_with_fields(
            false,
            &[
                field_fixed(200, b'y', &[7]),
                field_fixed(201, b't', &[0xff; 8]),
                field_u32(5, read_one.serial, false),
                field_typed(8, "v", "g", false),
            ],
            2,
        );
        bus.reply_variant(false, read_one.serial, 7);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "the unknown fields are skipped and the serial behind them is found"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_field_read_from_its_padding_does_not_answer_a_call() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // The truncated field names no serial; the padding hides one. A
        // parser that reads past the declared block would take this frame
        // and its value, so the honest answer follows it.
        let read_one = bus.read_call();
        bus.reply_truncated(read_one.serial, 7);
        bus.reply_variant(false, read_one.serial, 2);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "only the declared field block answers the call"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_change_to_another_setting_is_skipped_not_fatal() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // The desktop changes an unrelated setting, whose value is a
        // string; then the color scheme changes.
        bus.signal_changed_string(
            "org.freedesktop.appearance",
            "accent-color",
            "rgb(53,109,237)",
        );
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 0);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.wait_for_changed(),
        Some((
            String::from("org.freedesktop.appearance"),
            String::from("color-scheme"),
            0
        )),
        "the unrelated change is skipped and the watch continues"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_signal_that_is_not_the_portal_s_shape_is_skipped() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty(add_match.serial);
        // Even with the portal's sender, a signal whose member, body
        // signature or path differ from the portal's carries no change.
        bus.send_signal(
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "PropertiesChanged",
            ":1.42",
            "ssv",
            &changed_body("org.freedesktop.appearance", "color-scheme", 7),
        );
        bus.send_signal(
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "SettingChanged",
            ":1.42",
            "ssu",
            &changed_body("org.freedesktop.appearance", "color-scheme", 7),
        );
        bus.send_signal(
            "/org/freedesktop/portal/other",
            "org.freedesktop.portal.Settings",
            "SettingChanged",
            ":1.42",
            "ssv",
            &changed_body("org.freedesktop.appearance", "color-scheme", 7),
        );
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.wait_for_changed(),
        Some((
            String::from("org.freedesktop.appearance"),
            String::from("color-scheme"),
            1
        )),
        "only the signal the portal's shape names delivers"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_reply_in_the_other_byte_order_is_read_by_its_own_order() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        let hello = bus.read_call();
        bus.reply_string_big_endian(hello.serial, ":1.42");
        let add_match = bus.read_call();
        bus.reply_empty_big_endian(add_match.serial);
        let read_one = bus.read_call();
        bus.reply_variant(true, read_one.serial, 2);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    connection.hello().expect("the bus names the client");
    connection
        .add_match("type='signal'")
        .expect("the rule is accepted");
    assert_eq!(
        connection.read_setting("org.freedesktop.appearance", "color-scheme"),
        Some(2),
        "a big-endian reply's integer reads as what it carries"
    );
    bus.join().expect("the fake bus ends");
}

#[test]
fn a_header_that_names_more_fields_than_any_message_holds_is_refused() {
    // The cap sits a little above the honest maximum; without it the
    // client would read this whole frame, treat it as an empty signal and
    // answer with the change that follows instead of refusing.
    let (client_side, bus_side) = socket_pair();
    thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        bus.send_frame((1 << 20) + 8, 0);
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    assert_eq!(
        connection.wait_for_changed(),
        None,
        "a header no allocation can honor ends the watch"
    );
}

#[test]
fn a_header_that_names_a_bigger_body_than_any_message_holds_is_refused() {
    // The same discrimination for the body cap: the frame's whole body is
    // written, so a client without the cap would read past it to the
    // change that follows.
    let (client_side, bus_side) = socket_pair();
    thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.accept_handshake();
        bus.send_frame(0, (16 << 20) + 8);
        bus.signal_changed("org.freedesktop.appearance", "color-scheme", 1);
    });

    let mut connection =
        Connection::authenticated(client_side, 1000).expect("the handshake answers");
    assert_eq!(
        connection.wait_for_changed(),
        None,
        "a body no allocation can honor ends the watch"
    );
}

#[test]
fn a_client_that_the_bus_rejects_connects_to_nothing() {
    let (client_side, bus_side) = socket_pair();
    let bus = thread::spawn(move || {
        let mut bus = FakeBus::over(bus_side);
        bus.read_sasl_line();
        bus.write_line("REJECTED EXTERNAL");
    });
    assert!(Connection::authenticated(client_side, 1000).is_none());
    bus.join().expect("the fake bus ends");
}

/// Builds the body of one `SettingChanged`: the namespace, the key and
/// the value in a variant.
fn changed_body(namespace: &str, key: &str, value: u32) -> Vec<u8> {
    let mut body = Vec::new();
    put_string(&mut body, namespace, false);
    put_string(&mut body, key, false);
    variant_u32_after(&mut body, value);
    body
}

/// Builds the body of one `SettingChanged` value: a variant that names an
/// unsigned 32-bit integer and carries it, laid out after whatever the
/// caller already built.
fn variant_u32_after(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(b"\x01u\0");
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Appends one string in the byte order `big_endian` names, apart from the
/// client's own encoder.
fn put_string(out: &mut Vec<u8>, value: &str, big_endian: bool) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    put_u32(out, value.len() as u32, big_endian);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

/// One header field: the code, then a variant that names the type
/// `variant_sig` holds and carries `value`.
fn field_typed(code: u8, value: &str, variant_sig: &str, big_endian: bool) -> Vec<u8> {
    let mut field = vec![code];
    field.push(variant_sig.len() as u8);
    field.extend_from_slice(variant_sig.as_bytes());
    field.push(0);
    match variant_sig.as_bytes() {
        // A signature answers with no alignment of its own.
        b"g" => {
            field.push(value.len() as u8);
            field.extend_from_slice(value.as_bytes());
            field.push(0);
        }
        _ => {
            while !field.len().is_multiple_of(4) {
                field.push(0);
            }
            put_u32(&mut field, value.len() as u32, big_endian);
            field.extend_from_slice(value.as_bytes());
            field.push(0);
        }
    }
    field
}

/// One header field whose variant carries an unsigned integer.
fn field_u32(code: u8, value: u32, big_endian: bool) -> Vec<u8> {
    let mut field = vec![code, 1, b'u', 0];
    while !field.len().is_multiple_of(4) {
        field.push(0);
    }
    put_u32(&mut field, value, big_endian);
    field
}

/// One header field whose variant holds a fixed-size basic type, its
/// value given as the bytes it lays out, aligned to its own size.
fn field_fixed(code: u8, signature: u8, value: &[u8]) -> Vec<u8> {
    let mut field = vec![code, 1, signature, 0];
    while !field.len().is_multiple_of(value.len()) {
        field.push(0);
    }
    field.extend_from_slice(value);
    field
}

/// Appends one integer in the byte order `big_endian` names.
fn put_u32(out: &mut Vec<u8>, value: u32, big_endian: bool) {
    let bytes = if big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    out.extend_from_slice(&bytes);
}

/// The first sixteen bytes of one message: the byte order, the kind, the
/// flags, the version, the body's length, the serial and the fields'
/// length, each integer in the order `big_endian` names.
fn message_header(
    endian: u8,
    kind: u8,
    serial: u32,
    body_len: u32,
    fields_len: u32,
    fields: &[u8],
    big_endian: bool,
) -> Vec<u8> {
    let mut message = vec![endian, kind, 0, 1];
    put_u32(&mut message, body_len, big_endian);
    put_u32(&mut message, serial, big_endian);
    put_u32(&mut message, fields_len, big_endian);
    message.extend_from_slice(fields);
    message
}

/// Assembles a header-field block, aligning each entry to the eight bytes
/// its structure alignment asks.
fn field_block(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut block = Vec::new();
    for entry in entries {
        while !block.len().is_multiple_of(8) {
            block.push(0);
        }
        block.extend_from_slice(entry);
    }
    block
}

/// One header field the fake bus read, its value in the type its
/// signature named.
#[derive(Debug, Clone)]
enum Field {
    /// A string.
    Str(String),
    /// An object path, whose layout agrees with a string's but whose type
    /// the protocol keeps apart.
    Path(String),
    /// A signature, which answers with no alignment of its own.
    Sig(String),
    /// An unsigned 32-bit integer.
    U32(u32),
}

/// One call the client sent, with what its header fields named.
#[derive(Debug)]
struct Call {
    /// The serial the client sent it under, which a reply must answer.
    serial: u32,
    /// The object path the call named.
    path: Option<String>,
    /// The bus name the call asked for, when it named one.
    destination: Option<String>,
    /// The interface the member lives on.
    interface: Option<String>,
    /// The method the call asked to run.
    member: String,
    /// The signature the call named for its body, when it carried one.
    body_signature: Option<String>,
    /// The body, whose strings `body_strings` walks.
    body: Vec<u8>,
}

impl Call {
    /// The call's body strings, read apart from the client's cursor.
    fn body_strings(&self) -> Vec<String> {
        let mut texts = Vec::new();
        let mut at = 0;
        while at + 4 <= self.body.len() {
            at += (4 - at % 4) % 4;
            let Some(chunk) = self.body.get(at..at + 4) else {
                break;
            };
            let len =
                u32::from_le_bytes(chunk.try_into().expect("the slice holds the length")) as usize;
            at += 4;
            let Some(text) = self.body.get(at..at + len) else {
                break;
            };
            at += len;
            if self.body.get(at) != Some(&0) {
                break;
            }
            at += 1;
            if let Ok(text) = String::from_utf8(text.to_vec()) {
                texts.push(text);
            }
        }
        texts
    }
}

/// A bus that pretends to be the session bus, speaking the protocol the
/// way the real one does: every message carries the header fields the
/// spec names, in the structure-of-code-and-variant layout.
struct FakeBus {
    stream: UnixStream,
}

impl FakeBus {
    fn over(stream: UnixStream) -> FakeBus {
        FakeBus { stream }
    }

    /// Reads the handshake up to and including `BEGIN`, checking the
    /// mechanism and the identity the client named, then answers the OK
    /// the bus sends when it accepts that user.
    fn accept_handshake(&mut self) {
        let auth = self.read_sasl_line();
        assert_eq!(
            auth.strip_prefix('\0').map(str::trim_end),
            Some("AUTH EXTERNAL 31303030"),
            "the client names the external mechanism and the hex of uid 1000"
        );
        self.write_line("OK 1234567890abcdef");
        let begin = self.read_sasl_line();
        assert_eq!(
            begin.trim_end(),
            "BEGIN",
            "the client ends the handshake before it speaks messages"
        );
    }

    /// Reads one line of the handshake, whose first carries the nul byte
    /// the client opens with.
    fn read_sasl_line(&mut self) -> String {
        let mut line = Vec::new();
        let mut byte = [0; 1];
        loop {
            std::io::Read::read_exact(&mut self.stream, &mut byte)
                .expect("the client speaks the handshake");
            if byte[0] == b'\n' {
                line.pop();
                return String::from_utf8(line).expect("the handshake speaks text");
            }
            line.push(byte[0]);
        }
    }

    /// Writes one line of the handshake.
    fn write_line(&mut self, line: &str) {
        use std::io::Write;
        self.stream
            .write_all(format!("{line}\r\n").as_bytes())
            .expect("the bus answers the handshake");
    }

    /// Reads one call the client sent, reading its header fields apart
    /// from the client's encoder.
    fn read_call(&mut self) -> Call {
        let mut head = [0; 16];
        std::io::Read::read_exact(&mut self.stream, &mut head).expect("the client sends a call");
        assert_eq!(head[0], b'l', "the client sends little-endian messages");
        assert_eq!(head[1], 1, "the client sends method calls");
        assert_eq!(head[3], 1, "the client speaks protocol version one");
        let body_len = u32::from_le_bytes(head[4..8].try_into().expect("the header holds it"));
        let serial = u32::from_le_bytes(head[8..12].try_into().expect("the header holds it"));
        let fields_len =
            u32::from_le_bytes(head[12..16].try_into().expect("the header holds it")) as usize;
        let padded_fields = fields_len.div_ceil(8) * 8;
        let mut rest = vec![0; padded_fields + body_len as usize];
        std::io::Read::read_exact(&mut self.stream, &mut rest)
            .expect("the call carries what its header names");
        let fields = read_fields(&rest[..padded_fields]);
        let body = rest.split_off(padded_fields);
        let integers = fields
            .iter()
            .filter_map(|(code, value)| match value {
                Field::U32(word) => Some((*code, *word)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            integers.is_empty(),
            "this client's calls carry no integer header fields: {integers:?}"
        );
        let take = |code: u8| {
            fields
                .iter()
                .find(|(named_code, _)| *named_code == code)
                .map(|(_, value)| value.clone())
        };
        let as_str = |value: Field| match value {
            Field::Str(text) => Some(text),
            _ => None,
        };
        let as_path = |value: Field| match value {
            Field::Path(text) => Some(text),
            _ => None,
        };
        let as_sig = |value: Field| match value {
            Field::Sig(text) => Some(text),
            _ => None,
        };
        Call {
            serial,
            path: take(1).and_then(as_path),
            destination: take(6).and_then(as_str),
            interface: take(2).and_then(as_str),
            member: take(3).and_then(as_str).expect("a call names its member"),
            body_signature: take(8).and_then(as_sig),
            body,
        }
    }

    /// Answers one call with a method return whose body holds one string.
    fn reply_string(&mut self, call_serial: u32, value: &str) {
        let mut body = Vec::new();
        put_string(&mut body, value, false);
        self.reply_message(false, call_serial, body, Some("s"));
    }

    /// The same answer, in the byte order a daemon that speaks
    /// big-endian would use.
    fn reply_string_big_endian(&mut self, call_serial: u32, value: &str) {
        let mut body = Vec::new();
        put_string(&mut body, value, true);
        self.reply_message(true, call_serial, body, Some("s"));
    }

    /// Answers one call with the empty body its method returns.
    fn reply_empty(&mut self, call_serial: u32) {
        self.reply_message(false, call_serial, Vec::new(), None);
    }

    /// The same answer, in big-endian order.
    fn reply_empty_big_endian(&mut self, call_serial: u32) {
        self.reply_message(true, call_serial, Vec::new(), None);
    }

    /// Answers one call with a body that holds one unsigned integer in a
    /// variant, as `ReadOne` does.
    fn reply_variant(&mut self, big_endian: bool, call_serial: u32, value: u32) {
        let mut body = vec![1, b'u', 0];
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        put_u32(&mut body, value, big_endian);
        self.reply_message(big_endian, call_serial, body, Some("v"));
    }

    /// Answers one call the way a version-one portal's `Read` does: the
    /// value's variant, wrapped in another variant that names it.
    fn reply_variant_in_variant(&mut self, big_endian: bool, call_serial: u32, value: u32) {
        let mut body = vec![1, b'v', 0];
        body.extend_from_slice(b"\x01u\0");
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        put_u32(&mut body, value, big_endian);
        self.reply_message(big_endian, call_serial, body, Some("v"));
    }

    /// Reports one call as failed, naming the method the bus refused.
    fn reply_error(&mut self, call_serial: u32) {
        let fields = field_block(&[
            field_u32(5, call_serial, false),
            field_typed(4, "org.freedesktop.DBus.Error.UnknownMethod", "s", false),
        ]);
        let mut message = message_header(b'l', 3, 9, 0, fields.len() as u32, &fields, false);
        while !message.len().is_multiple_of(8) {
            message.push(0);
        }
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus refuses the call");
    }

    /// Writes one method return, whose header fields name the call it
    /// answers and the type of the body it carries, in the byte order
    /// `big_endian` names.
    fn reply_message(
        &mut self,
        big_endian: bool,
        call_serial: u32,
        mut body: Vec<u8>,
        body_signature: Option<&str>,
    ) {
        let mut entries = vec![field_u32(5, call_serial, big_endian)];
        if let Some(signature) = body_signature {
            entries.push(field_typed(8, signature, "g", big_endian));
        }
        let fields = field_block(&entries);
        let mut message = message_header(
            if big_endian { b'B' } else { b'l' },
            2,
            9,
            body.len() as u32,
            fields.len() as u32,
            &fields,
            big_endian,
        );
        while !message.len().is_multiple_of(8) {
            message.push(0);
        }
        message.append(&mut body);
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus answers the call");
    }

    /// Answers one call with the body signature field named before the
    /// reply serial, the order a hostile peer might try, carrying one
    /// unsigned integer in a variant.
    fn reply_variant_signature_first(&mut self, big_endian: bool, call_serial: u32, value: u32) {
        self.reply_variant_with_fields(
            big_endian,
            &[
                field_typed(8, "v", "g", big_endian),
                field_u32(5, call_serial, big_endian),
            ],
            value,
        );
    }

    /// Writes one method return whose header holds exactly the field
    /// entries given, carrying one unsigned integer in a variant.
    fn reply_variant_with_fields(&mut self, big_endian: bool, entries: &[Vec<u8>], value: u32) {
        let mut body = vec![1, b'u', 0];
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        put_u32(&mut body, value, big_endian);
        let fields = field_block(entries);
        let mut message = message_header(
            if big_endian { b'B' } else { b'l' },
            2,
            9,
            body.len() as u32,
            fields.len() as u32,
            &fields,
            big_endian,
        );
        while !message.len().is_multiple_of(8) {
            message.push(0);
        }
        message.extend_from_slice(&body);
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus answers the call");
    }

    /// Answers one call with a field block that declares less than its
    /// alignment padding carries: the reply serial hides in the padding,
    /// which no honest message does.
    fn reply_truncated(&mut self, call_serial: u32, value: u32) {
        let mut body = vec![1, b'u', 0];
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        put_u32(&mut body, value, false);
        let mut message = vec![b'l', 2, 0, 1];
        put_u32(&mut message, body.len() as u32, false);
        put_u32(&mut message, 9, false);
        put_u32(&mut message, 4, false);
        // The declared field stops after its type; the serial sits in the
        // padding past it.
        message.extend_from_slice(&[5, 1, b'u', 0]);
        message.extend_from_slice(&call_serial.to_le_bytes());
        message.extend_from_slice(&body);
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus answers the call");
    }

    /// Writes one frame the client's caps refuse, header and whole body,
    /// so a client without the caps reads past it to whatever follows.
    fn send_frame(&mut self, fields_len: u32, body_len: u32) {
        let message = message_header(b'l', 4, 1, body_len, fields_len, &[], false);
        std::io::Write::write_all(&mut self.stream, &message)
            .expect("the broken header leaves the bus");
        let rest_len = (fields_len as usize).div_ceil(8) * 8 + body_len as usize;
        std::io::Write::write_all(&mut self.stream, &vec![0; rest_len])
            .expect("the broken frame's body leaves the bus");
    }

    /// Broadcasts one `SettingChanged` signal, whose header fields name
    /// the path, interface and member a real one carries, sent by the
    /// portal's owner.
    fn signal_changed(&mut self, namespace: &str, key: &str, value: u32) {
        self.send_signal(
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "SettingChanged",
            ":1.42",
            "ssv",
            &changed_body(namespace, key, value),
        );
    }

    /// Sends one `SettingChanged` addressed to the client by name, which a
    /// broadcast never is: any connection can send one, past the match rule.
    fn signal_changed_to_client(&mut self, value: u32) {
        let body = changed_body("org.freedesktop.appearance", "color-scheme", value);
        let fields = field_block(&[
            field_typed(1, "/org/freedesktop/portal/desktop", "o", false),
            field_typed(2, "org.freedesktop.portal.Settings", "s", false),
            field_typed(3, "SettingChanged", "s", false),
            field_typed(6, ":1.42", "s", false),
            field_typed(7, ":1.99", "s", false),
            field_typed(8, "ssv", "g", false),
        ]);
        let mut message = message_header(
            b'l',
            4,
            4,
            body.len() as u32,
            fields.len() as u32,
            &fields,
            false,
        );
        while !message.len().is_multiple_of(8) {
            message.push(0);
        }
        message.extend_from_slice(&body);
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus forwards the signal");
    }

    /// Broadcasts one `SettingChanged` whose value is a string, as a
    /// change to an unrelated setting arrives.
    fn signal_changed_string(&mut self, namespace: &str, key: &str, value: &str) {
        let mut body = Vec::new();
        put_string(&mut body, namespace, false);
        put_string(&mut body, key, false);
        body.extend_from_slice(b"\x01s\0");
        put_string(&mut body, value, false);
        self.send_signal(
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "SettingChanged",
            ":1.42",
            "ssv",
            &body,
        );
    }

    /// Writes one signal, whose header fields name the path, interface,
    /// member, sender and body signature given, so a test can send one a
    /// real portal would never emit and see the client refuse it.
    fn send_signal(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        sender: &str,
        body_signature: &str,
        body: &[u8],
    ) {
        let fields = field_block(&[
            field_typed(1, path, "o", false),
            field_typed(2, interface, "s", false),
            field_typed(3, member, "s", false),
            field_typed(7, sender, "s", false),
            field_typed(8, body_signature, "g", false),
        ]);
        let mut message = message_header(
            b'l',
            4,
            3,
            body.len() as u32,
            fields.len() as u32,
            &fields,
            false,
        );
        while !message.len().is_multiple_of(8) {
            message.push(0);
        }
        message.extend_from_slice(body);
        std::io::Write::write_all(&mut self.stream, &message).expect("the bus forwards the signal");
    }
}

/// Reads a header-field block: an array of structures of a code byte and
/// a variant, read apart from the client's encoder.
fn read_fields(bytes: &[u8]) -> Vec<(u8, Field)> {
    let mut fields = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        at += (8 - at % 8) % 8;
        let Some(code) = bytes.get(at).copied() else {
            break;
        };
        at += 1;
        let Some((signature, next)) = read_signature(bytes, at) else {
            break;
        };
        at = next;
        let (value, next) = match signature.as_str() {
            "s" | "o" => {
                at += (4 - at % 4) % 4;
                let Some(len) = u32_at(bytes, at) else { break };
                let text = bytes
                    .get(at + 4..at + 4 + len as usize)
                    .and_then(|text| String::from_utf8(text.to_vec()).ok());
                // The nul the protocol counts trails the text, and no
                // honest string carries one inside.
                let nul = bytes.get(at + 4 + len as usize).copied() == Some(0);
                let value = match (
                    signature.as_str(),
                    text.filter(|text| nul && !text.contains('\0')),
                ) {
                    ("o", Some(text)) => Some(Field::Path(text)),
                    ("s", Some(text)) => Some(Field::Str(text)),
                    _ => None,
                };
                (value, at + 4 + len as usize + 1)
            }
            "g" => {
                let Some((text, next)) = read_signature(bytes, at) else {
                    break;
                };
                (Some(Field::Sig(text)), next)
            }
            "u" => {
                at += (4 - at % 4) % 4;
                (u32_at(bytes, at).map(Field::U32), at + 4)
            }
            _ => break,
        };
        let Some(value) = value else { break };
        fields.push((code, value));
        at = next;
    }
    fields
}

/// Reads one signature: its length byte, then the text, then the nul the
/// protocol counts.
fn read_signature(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    let len = *bytes.get(at)? as usize;
    let text = bytes.get(at + 1..at + 1 + len)?;
    if *bytes.get(at + 1 + len)? != 0 {
        return None;
    }
    let text = String::from_utf8(text.to_vec()).ok()?;
    if text.contains('\0') {
        return None;
    }
    Some((text, at + len + 2))
}

/// Reads one unsigned integer at `at`, which the block holds little-
/// endian here.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let chunk = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes(
        chunk.try_into().expect("the slice holds it"),
    ))
}
