//! A small D-Bus client, just enough to watch one setting of the session
//! bus.
//!
//! The client speaks the wire protocol by hand, because the crates that
//! speak all of D-Bus pull in an async runtime, which this crate keeps out.
//! It knows one side of the protocol: it names itself over the SASL
//! external handshake, sends method calls whose bodies hold strings and
//! unsigned 32-bit integers, and reads back replies and signals of the same
//! few types, which is all the settings portal needs.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// The message that asks a peer to run one of its methods.
const METHOD_CALL: u8 = 1;

/// The message that answers a method call.
const METHOD_RETURN: u8 = 2;

/// The message that reports a method call as failed.
const ERROR: u8 = 3;

/// The message a peer broadcasts to whoever subscribed to it.
const SIGNAL: u8 = 4;

/// The largest body this client reads, which keeps a broken header from
/// making it allocate without bound.
const MAX_BODY: u32 = 16 << 20;

/// The largest header-field block this client reads, which no honest
/// message comes near and a broken header cannot grow past.
const MAX_FIELDS: usize = 1 << 20;

/// The bus name of the settings portal, which serves the desktop's settings.
const PORTAL: &str = "org.freedesktop.portal.Desktop";

/// The interface of the settings portal, on the desktop object at its
/// standard path.
const SETTINGS: &str = "org.freedesktop.portal.Settings";

/// The object path the settings portal serves.
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// The member of the signal the portal broadcasts when a setting changes.
const SETTING_CHANGED: &str = "SettingChanged";

/// The body signature of the settings-changed signal: the namespace, the
/// key and the value as a variant.
const CHANGED_SIGNATURE: &str = "ssv";

/// One open conversation with the session bus.
pub(crate) struct Connection {
    /// The socket the bus answers on.
    stream: UnixStream,
    /// The serial of the last message this client sent, which its next one
    /// follows.
    serial: u32,
    /// The setting [`Connection::read_settled`] reads, whose changes a call
    /// notes while it waits for its reply.
    watched: Option<(String, String)>,
    /// The last change of the watched setting announced while a call
    /// waited.
    raced: Option<u32>,
}

/// How many times [`Connection::read_settled`] reads again after a change
/// raced its read.
const SETTLE_READS: usize = 3;

impl Connection {
    /// Connects to the session bus the environment names, or `None` when it
    /// names nothing this client can speak to, for example an abstract
    /// socket address. A candidate whose socket answers but whose
    /// handshake fails does not stop the later candidates.
    pub(crate) fn connect() -> Option<Connection> {
        let addresses = session_addresses(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        );
        for path in addresses {
            if let Ok(stream) = UnixStream::connect(&path)
                // The external authentication names the user this process
                // runs as, which is the user the bus serves.
                && let Some(connection) =
                    Connection::authenticated(stream, rustix::process::geteuid().as_raw())
            {
                return Some(connection);
            }
        }
        None
    }

    /// Builds a connection on a socket that is already connected, naming
    /// the user `uid` to the bus over the handshake.
    fn authenticated(stream: UnixStream, uid: u32) -> Option<Connection> {
        let mut connection = Connection {
            stream,
            serial: 0,
            watched: None,
            raced: None,
        };
        connection.authenticate(uid)?;
        Some(connection)
    }

    /// Names this client to the bus over the SASL external mechanism, whose
    /// identity is the user the client runs as.
    fn authenticate(&mut self, uid: u32) -> Option<()> {
        let hex = |text: &str| {
            text.bytes()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>()
        };
        self.stream.write_all(b"\0").ok()?;
        self.stream
            .write_all(format!("AUTH EXTERNAL {}\r\n", hex(&uid.to_string())).as_bytes())
            .ok()?;
        let reply = self.read_line()?;
        if !reply.starts_with("OK") {
            return None;
        }
        self.stream.write_all(b"BEGIN\r\n").ok()?;
        Some(())
    }

    /// Reads one line of the handshake, which ends in a carriage return and
    /// a line feed.
    fn read_line(&mut self) -> Option<String> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0; 1];
            self.stream.read_exact(&mut byte).ok()?;
            if byte[0] == b'\n' {
                line.pop();
                return String::from_utf8(line).ok();
            }
            line.push(byte[0]);
            if line.len() > 128 {
                return None;
            }
        }
    }

    /// Sends one method call and waits for what answers it. A change of the
    /// watched setting announced meanwhile is noted, the last one, for
    /// [`Connection::read_settled`] to learn that a change raced its read.
    /// Anything else that arrives is skipped. An error reply answers `None`,
    /// like a bus that breaks.
    fn call(
        &mut self,
        path: &str,
        destination: &str,
        interface: &str,
        member: &str,
        body: &[u8],
        body_signature: &str,
    ) -> Option<Message> {
        self.serial += 1;
        let message = call_message(
            self.serial,
            path,
            destination,
            interface,
            member,
            body,
            body_signature,
        );
        self.stream.write_all(&message).ok()?;
        loop {
            let message = read_message(&mut self.stream)?;
            // A reply names the call it answers; anything else on the
            // connection, or a reply to some earlier serial, is not what
            // this call waits for.
            match message.kind {
                METHOD_RETURN if message.fields.reply_serial == Some(self.serial) => {
                    return Some(message);
                }
                ERROR if message.fields.reply_serial == Some(self.serial) => return None,
                _ => {
                    if let Some((namespace, key, value)) = message.setting_change()
                        && self.watched.as_ref() == Some(&(namespace, key))
                    {
                        self.raced = Some(value);
                    }
                }
            }
        }
    }

    /// Reads one setting of the settings portal and keeps reading while a
    /// change of it races the read, answering the value to start from.
    ///
    /// A change announced while the read waits may come from the portal that
    /// answers or from one that replaced it, before or after the reply, and
    /// the bus orders only each sender's own messages. A read that follows
    /// goes to whoever owns the portal then, so its answer settles the race.
    /// When a read fails, or every read raced a change, the last change seen
    /// during any of the reads is the value to go by: from the portal that answered, it equals what
    /// that portal's reply held, since a portal announces a change before
    /// it serves a read of it, and from a portal that replaced it, it is
    /// newer. With no change seen, the answer of an earlier read stands.
    pub(crate) fn read_settled(&mut self, namespace: &str, key: &str) -> Option<u32> {
        self.watched = Some((String::from(namespace), String::from(key)));
        let mut answer = None;
        let mut last_change = None;
        let mut settled = None;
        for _ in 0..SETTLE_READS {
            let read = self.read_setting(namespace, key);
            let changed = self.raced.take();
            last_change = changed.or(last_change);
            match (read, changed) {
                (Some(value), None) => {
                    settled = Some(value);
                    break;
                }
                (Some(value), Some(_)) => answer = Some(value),
                (None, _) => break,
            }
        }
        self.watched = None;
        settled.or(last_change).or(answer)
    }

    /// Names this client to the bus, which every conversation starts with.
    pub(crate) fn hello(&mut self) -> Option<()> {
        self.call(
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "org.freedesktop.DBus",
            "Hello",
            &[],
            "",
        )
        .map(|_| ())
    }

    /// Asks the bus to forward every signal the rule names.
    pub(crate) fn add_match(&mut self, rule: &str) -> Option<()> {
        let mut body = Body::default();
        body.string(rule);
        self.call(
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "org.freedesktop.DBus",
            "AddMatch",
            &body.0,
            "s",
        )
        .map(|_| ())
    }

    /// Reads one setting of the settings portal, answering its value.
    ///
    /// A version-two portal answers `ReadOne`, whose reply holds the
    /// value's own variant. A version-one portal has no `ReadOne`, and its
    /// `Read` answers the value wrapped in another variant, which names
    /// it; the older form is tried when the newer names no method.
    fn read_setting(&mut self, namespace: &str, key: &str) -> Option<u32> {
        let mut body = Body::default();
        body.string(namespace);
        body.string(key);
        if let Some(reply) = self.call(PORTAL_PATH, PORTAL, SETTINGS, "ReadOne", &body.0, "ss")
            && let Some(value) = Cursor::over(&reply.body, reply.big_endian).variant_u32()
        {
            return Some(value);
        }
        let reply = self.call(PORTAL_PATH, PORTAL, SETTINGS, "Read", &body.0, "ss")?;
        let mut cursor = Cursor::over(&reply.body, reply.big_endian);
        if cursor.signature().as_deref() != Some("v") {
            return None;
        }
        cursor.variant_u32()
    }

    /// Waits for the next change the portal announces, answering the
    /// setting that changed and its new value.
    ///
    /// Only the portal's broadcasts reach this connection, because the match
    /// rule names the portal as sender and the bus forwards a broadcast
    /// that names a well-known sender only from that name's current owner;
    /// a signal addressed to this connection is not the portal's and is
    /// skipped like a message of any other shape. Only a stream that breaks
    /// ends the wait.
    pub(crate) fn wait_for_changed(&mut self) -> Option<(String, String, u32)> {
        loop {
            let message = read_message(&mut self.stream)?;
            if let Some(change) = message.setting_change() {
                return Some(change);
            }
        }
    }
}

/// The session bus sockets to try, given the bus address and the user's
/// runtime directory the environment holds.
///
/// Only address candidates that name a unix path are answered. When the
/// address names none, for example because it holds only abstract sockets
/// or a temporary directory, or when there is no address at all, the
/// well-known place under the runtime directory is tried instead.
fn session_addresses(
    address: Option<&str>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> Vec<std::path::PathBuf> {
    let named: Vec<_> = address
        .into_iter()
        .flat_map(|address| address.split(';'))
        .filter_map(candidate_path)
        .collect();
    if !named.is_empty() {
        return named;
    }
    runtime_dir
        .map(|dir| std::path::Path::new(dir).join("bus"))
        .into_iter()
        .collect()
}

/// The path of one address candidate, a `unix:` transport whose
/// comma-separated properties may name `path=` in any order, with its
/// escapes decoded.
fn candidate_path(candidate: &str) -> Option<std::path::PathBuf> {
    let properties = candidate.strip_prefix("unix:")?;
    let path = properties
        .split(',')
        .find_map(|property| property.strip_prefix("path="))?;
    percent_decode(path)
}

/// Decodes the `%XX` escapes an address may hold, or `None` when the text
/// is not a path once decoded. A path is bytes, not text, so a decoded
/// byte that is no character still names a socket.
fn percent_decode(text: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    Some(std::ffi::OsString::from_vec(decoded).into())
}

/// One message the bus sent, with what it carries in its body.
struct Message {
    /// Why the bus sent it: a reply, an error or a signal.
    kind: u8,
    /// Whether the body's integers are big-endian, which each message
    /// declares for itself.
    big_endian: bool,
    /// The header fields this client reads, among those the message
    /// carried.
    fields: Fields,
    /// The body, whose type the message's interface fixes.
    body: Vec<u8>,
}

/// The header fields this client reads out of a message.
#[derive(Default)]
struct Fields {
    /// The serial of the call a reply answers.
    reply_serial: Option<u32>,
    /// The object path a signal was emitted on.
    path: Option<String>,
    /// The interface a signal was emitted on.
    interface: Option<String>,
    /// The signal's member.
    member: Option<String>,
    /// The types the body carries.
    body_signature: Option<String>,
    /// Whether the message names a destination, which a broadcast signal
    /// never does.
    addressed: bool,
}

impl Message {
    /// The change this signal carries, when it is shaped as the portal's
    /// signal: broadcast on the portal's path and interface, named
    /// `SettingChanged`, and holding a namespace, a key and a value.
    ///
    /// The match rule limits broadcasts to the portal's owner, but a signal
    /// sent to this connection by name reaches it whatever the rule says,
    /// and the portal only ever broadcasts, so an addressed one is not its.
    fn setting_change(&self) -> Option<(String, String, u32)> {
        if self.kind != SIGNAL || self.fields.addressed {
            return None;
        }
        let named = self.fields.path.as_deref() == Some(PORTAL_PATH)
            && self.fields.interface.as_deref() == Some(SETTINGS)
            && self.fields.member.as_deref() == Some(SETTING_CHANGED)
            && self.fields.body_signature.as_deref() == Some(CHANGED_SIGNATURE);
        if !named {
            return None;
        }
        let mut cursor = Cursor::over(&self.body, self.big_endian);
        let namespace = cursor.string()?;
        let key = cursor.string()?;
        let value = cursor.variant_u32()?;
        Some((namespace, key, value))
    }
}

/// Reads one message, whose header says how much follows it.
fn read_message(stream: &mut UnixStream) -> Option<Message> {
    let mut head = [0; 16];
    stream.read_exact(&mut head).ok()?;
    let big_endian = match head[0] {
        b'l' => false,
        b'B' => true,
        _ => return None,
    };
    let body_len = u32_at(&head[4..8], big_endian);
    if body_len > MAX_BODY {
        return None;
    }
    let fields_len = u32_at(&head[12..16], big_endian) as usize;
    if fields_len > MAX_FIELDS {
        return None;
    }
    let padded_fields = fields_len.div_ceil(8) * 8;
    let rest_len = padded_fields.checked_add(body_len as usize)?;
    let mut rest = vec![0; rest_len];
    stream.read_exact(&mut rest).ok()?;
    // Only the declared field block is parsed; what follows it in the
    // padding, and the body after it, is not field data.
    let fields = header_fields(&rest[..fields_len], big_endian);
    let body = rest.split_off(padded_fields);
    Some(Message {
        kind: head[1],
        big_endian,
        fields,
        body,
    })
}

/// Reads the header fields this client answers for, whose entries are
/// structures of a code byte and a variant. A field this client does not
/// name is skipped by its type; a block that stops parsing partway
/// answers with the fields read so far, whose absence the callers treat
/// as the message saying nothing.
///
/// Every field the protocol defines holds a basic type, so any basic type
/// is skipped. A field that holds a container would need a full walk of
/// the marshalling format to skip, so parsing stops there instead, and the
/// message still answers with the fields read before it.
fn header_fields(fields: &[u8], big_endian: bool) -> Fields {
    let mut read = Fields::default();
    let mut at = 0;
    while at < fields.len() {
        at += (8 - at % 8) % 8;
        let Some(code) = fields.get(at).copied() else {
            break;
        };
        at += 1;
        // The variant names its type: a length byte, the text, and the
        // nul the protocol counts.
        let Some(len) = fields.get(at).copied() else {
            break;
        };
        let Some(signature) = fields.get(at + 1..at + 1 + len as usize) else {
            break;
        };
        if fields.get(at + 1 + len as usize).copied() != Some(0) {
            break;
        }
        at += 1 + len as usize + 1;
        let Ok(signature) = std::str::from_utf8(signature) else {
            break;
        };
        match signature {
            "u" => {
                at += (4 - at % 4) % 4;
                let Some(chunk) = fields.get(at..at + 4) else {
                    break;
                };
                at += 4;
                // Field 5 names the call a reply answers.
                if code == 5 {
                    read.reply_serial = Some(u32_at(chunk, big_endian));
                }
            }
            "g" => {
                let Some(len) = fields.get(at).copied() else {
                    break;
                };
                let text = fields.get(at + 1..at + 1 + len as usize);
                let ended = fields.get(at + 1 + len as usize).copied() == Some(0);
                let value = text
                    .and_then(|text| String::from_utf8(text.to_vec()).ok())
                    .filter(|text| ended && !text.contains('\0'));
                at += 1 + len as usize + 1;
                if code == 8 {
                    read.body_signature = value;
                }
            }
            "s" | "o" => {
                at += (4 - at % 4) % 4;
                let Some(chunk) = fields.get(at..at + 4) else {
                    break;
                };
                let len = u32_at(chunk, big_endian) as usize;
                // The length is a wire number, so nothing is sliced until
                // its arithmetic holds.
                let Some(text_at) = at.checked_add(4) else {
                    break;
                };
                let Some(end) = text_at.checked_add(len) else {
                    break;
                };
                let Some(text) = fields.get(text_at..end) else {
                    break;
                };
                if fields.get(end).copied() != Some(0) {
                    break;
                }
                at = end + 1;
                let value = String::from_utf8(text.to_vec())
                    .ok()
                    .filter(|text| !text.contains('\0'));
                if code == 6 {
                    read.addressed = true;
                }
                match (code, value) {
                    (1, Some(path)) => read.path = Some(path),
                    (2, Some(interface)) => read.interface = Some(interface),
                    (3, Some(member)) => read.member = Some(member),
                    _ => {}
                }
            }
            other => {
                let Some(size) = fixed_size(other) else {
                    break;
                };
                at += (size - at % size) % size;
                at += size;
            }
        }
    }
    read
}

/// The size of one fixed-size basic type, which is also its alignment, or
/// `None` for any other type.
fn fixed_size(signature: &str) -> Option<usize> {
    match signature {
        "y" => Some(1),
        "n" | "q" => Some(2),
        "b" | "i" | "u" | "h" => Some(4),
        "x" | "t" | "d" => Some(8),
        _ => None,
    }
}

/// Reads a little- or big-endian integer out of a message header.
fn u32_at(bytes: &[u8], big_endian: bool) -> u32 {
    let chunk: [u8; 4] = bytes.try_into().expect("the header holds the integer");
    if big_endian {
        u32::from_be_bytes(chunk)
    } else {
        u32::from_le_bytes(chunk)
    }
}

/// Builds one method call: its header fields name where it goes, and its
/// body carries the arguments, which travel after a padded-to-eight header.
fn call_message(
    serial: u32,
    path: &str,
    destination: &str,
    interface: &str,
    member: &str,
    body: &[u8],
    body_signature: &str,
) -> Vec<u8> {
    let mut fields = Vec::new();
    field(&mut fields, 1, path, "o");
    field(&mut fields, 6, destination, "s");
    field(&mut fields, 2, interface, "s");
    field(&mut fields, 3, member, "s");
    if !body_signature.is_empty() {
        field(&mut fields, 8, body_signature, "g");
    }
    let mut message = vec![
        b'l',
        METHOD_CALL,
        // The NO_REPLY_EXPECTED flag stays off: every call here waits.
        0,
        1,
    ];
    message.extend_from_slice(&(body.len() as u32).to_le_bytes());
    message.extend_from_slice(&serial.to_le_bytes());
    message.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    message.append(&mut fields);
    while !message.len().is_multiple_of(8) {
        message.push(0);
    }
    message.extend_from_slice(body);
    message
}

/// One method call's arguments, built in the order the interface names.
#[derive(Default)]
struct Body(
    /// The bytes, which the arguments' types lay out.
    Vec<u8>,
);

impl Body {
    /// Adds one string, whose length leads and whose end carries the nul
    /// byte the protocol counts.
    fn string(&mut self, value: &str) {
        align(&mut self.0, 4);
        self.0
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(0);
    }
}

/// Adds one header field, which is a structure of the field's code and a
/// variant holding `value` as the type `signature` names.
///
/// A variant in the D-Bus format starts right where the last byte ended; it
/// is its signature, and then the value with the type's own alignment.
fn field(fields: &mut Vec<u8>, code: u8, value: &str, signature: &str) {
    align(fields, 8);
    fields.push(code);
    push_signature(fields, signature);
    match signature {
        // A signature answers with no alignment of its own.
        "g" => push_signature(fields, value),
        _ => {
            align(fields, 4);
            push_string(fields, value);
        }
    }
}

fn align(out: &mut Vec<u8>, to: usize) {
    while !out.len().is_multiple_of(to) {
        out.push(0);
    }
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

fn push_signature(out: &mut Vec<u8>, value: &str) {
    out.push(value.len() as u8);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

/// Reads the arguments of one message body out of the bytes the message
/// carried, keeping to the alignments the protocol fixes per type.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    big_endian: bool,
}

impl<'a> Cursor<'a> {
    /// Starts a cursor over one body, whose offsets begin at zero however
    /// the message's header sat.
    fn over(bytes: &'a [u8], big_endian: bool) -> Cursor<'a> {
        Cursor {
            bytes,
            at: 0,
            big_endian,
        }
    }

    fn align(&mut self, to: usize) {
        self.at += (to - self.at % to) % to;
    }

    fn u8(&mut self) -> Option<u8> {
        let byte = self.bytes.get(self.at).copied()?;
        self.at += 1;
        Some(byte)
    }

    fn u32(&mut self) -> Option<u32> {
        self.align(4);
        let chunk = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        let chunk: [u8; 4] = chunk.try_into().expect("the slice holds the integer");
        Some(if self.big_endian {
            u32::from_be_bytes(chunk)
        } else {
            u32::from_le_bytes(chunk)
        })
    }

    /// Reads one string, whose length leads and which ends in a nul byte.
    /// A nul inside the text is not part of any string the protocol
    /// carries, and a length that overflows the body names no string.
    fn string(&mut self) -> Option<String> {
        let length = self.u32()? as usize;
        let end = self.at.checked_add(length)?;
        let text = self.bytes.get(self.at..end)?;
        self.at = end;
        if self.u8()? != 0 {
            return None;
        }
        let text = String::from_utf8(text.to_vec()).ok()?;
        if text.contains('\0') {
            return None;
        }
        Some(text)
    }

    /// Reads one type name, which leads every variant.
    fn signature(&mut self) -> Option<String> {
        let length = self.u8()? as usize;
        let end = self.at.checked_add(length)?;
        let text = self.bytes.get(self.at..end)?;
        self.at = end;
        if self.u8()? != 0 {
            return None;
        }
        let text = String::from_utf8(text.to_vec()).ok()?;
        if text.contains('\0') {
            return None;
        }
        Some(text)
    }

    /// Reads one variant, whose only form this client answers for is an
    /// unsigned 32-bit integer.
    ///
    /// A variant in the D-Bus format starts right where the last byte
    /// ended; its integer value still keeps the four-byte alignment the
    /// type carries everywhere.
    fn variant_u32(&mut self) -> Option<u32> {
        let signature = self.signature()?;
        if signature != "u" {
            return None;
        }
        self.u32()
    }
}

#[cfg(test)]
#[path = "dbus_tests.rs"]
mod tests;
