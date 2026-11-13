use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{BestMime, decode, read_to_end, serve, write_within};

const SHORT: Duration = Duration::from_millis(100);

fn best_of(offered: &[&str]) -> Option<String> {
    let best = BestMime::new();
    for mime in offered {
        best.advertise(mime);
    }
    best.get()
}

#[test]
fn paste_prefers_utf8_text_whatever_order_it_is_advertised_in() {
    let offered = [
        "image/png",
        "STRING",
        "TEXT",
        "text/plain;charset=utf-8",
        "UTF8_STRING",
    ];
    assert_eq!(
        best_of(&offered).as_deref(),
        Some("text/plain;charset=utf-8")
    );
}

#[test]
fn paste_falls_back_through_the_text_types_in_order() {
    assert_eq!(
        best_of(&["text/plain", "UTF8_STRING"]).as_deref(),
        Some("UTF8_STRING")
    );
    assert_eq!(
        best_of(&["STRING", "text/plain"]).as_deref(),
        Some("text/plain")
    );
    assert_eq!(best_of(&["STRING", "TEXT"]).as_deref(), Some("TEXT"));
    assert_eq!(best_of(&["text/html", "STRING"]).as_deref(), Some("STRING"));
}

#[test]
fn paste_matches_text_types_regardless_of_case_and_asks_in_the_offer_spelling() {
    assert_eq!(
        best_of(&["text/plain;charset=UTF-8"]).as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    assert_eq!(
        best_of(&["text/plain", "TEXT/PLAIN;CHARSET=UTF-8"]).as_deref(),
        Some("TEXT/PLAIN;CHARSET=UTF-8")
    );
}

#[test]
fn paste_finds_nothing_in_an_offer_without_text() {
    assert_eq!(best_of(&["image/png", "text/html", "text/uri-list"]), None);
    assert_eq!(best_of(&[]), None);
}

#[test]
fn read_returns_everything_written_before_the_pipe_closes() {
    let (reader, mut writer) = std::io::pipe().expect("a pipe opens");
    let text = "x".repeat(1 << 20);
    let sent = text.clone();
    let writing = std::thread::spawn(move || writer.write_all(sent.as_bytes()));
    assert_eq!(
        read_to_end(reader, Duration::from_secs(10)),
        Some(text.into_bytes())
    );
    writing
        .join()
        .expect("the writer finishes")
        .expect("the write succeeds");
}

#[test]
fn read_of_a_closed_empty_pipe_is_empty_text() {
    let (reader, writer) = std::io::pipe().expect("a pipe opens");
    drop(writer);
    assert_eq!(read_to_end(reader, SHORT), Some(Vec::new()));
}

#[test]
fn read_gives_up_on_a_writer_that_never_closes() {
    let (reader, mut writer) = std::io::pipe().expect("a pipe opens");
    writer.write_all(b"partial").expect("the write succeeds");
    let start = Instant::now();
    assert_eq!(read_to_end(reader, SHORT), None);
    assert!(start.elapsed() < Duration::from_secs(5));
    drop(writer);
}

#[test]
fn read_gives_up_on_a_writer_that_never_writes() {
    let (reader, writer) = std::io::pipe().expect("a pipe opens");
    assert_eq!(read_to_end(reader, SHORT), None);
    drop(writer);
}

#[test]
fn copy_serves_its_text_and_closes_the_pipe() {
    let (reader, writer) = std::io::pipe().expect("a pipe opens");
    serve(Arc::from("copied text"), OwnedFd::from(writer));
    assert_eq!(
        read_to_end(reader, Duration::from_secs(10)),
        Some(b"copied text".to_vec())
    );
}

#[test]
fn copy_serves_all_of_a_large_text_through_a_non_blocking_pipe() {
    let (reader, writer) = std::io::pipe().expect("a pipe opens");
    let writer = OwnedFd::from(writer);
    rustix::io::ioctl_fionbio(&writer, true).expect("the pipe turns non-blocking");
    let text: String = "0123456789abcdef".repeat(16 * 1024);
    serve(Arc::from(text.as_str()), writer);
    assert_eq!(
        read_to_end(reader, Duration::from_secs(10)),
        Some(text.into_bytes())
    );
}

#[test]
fn copy_gives_up_on_a_reader_that_stops_reading() {
    let (_reader, writer) = std::io::pipe().expect("a pipe opens");
    let writer = OwnedFd::from(writer);
    let started = Instant::now();
    let written = write_within(&writer, &[0; 1024 * 1024], Duration::from_millis(100));
    assert!(!written, "the pipe never had room for the whole text");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn copy_keeps_serving_a_slow_reader_that_keeps_reading() {
    let (mut reader, writer) = std::io::pipe().expect("a pipe opens");
    let writer = OwnedFd::from(writer);
    let reading = std::thread::spawn(move || {
        let mut total = 0;
        let mut chunk = [0; 64 * 1024];
        loop {
            std::thread::sleep(Duration::from_millis(40));
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => return total,
                Ok(read) => total += read,
            }
        }
    });
    let started = Instant::now();

    let written = write_within(&writer, &[0; 1024 * 1024], Duration::from_millis(200));

    assert!(written, "a reader that keeps reading gets the whole text");
    assert!(
        started.elapsed() > Duration::from_millis(200),
        "the copy outlasted one stall period"
    );
    drop(writer);
    assert_eq!(reading.join().expect("the reader finishes"), 1024 * 1024);
}

#[test]
fn paste_replaces_invalid_utf8_instead_of_refusing_it() {
    assert_eq!(decode(b"caf\xe9 ok".to_vec()), "caf\u{fffd} ok");
    assert_eq!(decode("café".as_bytes().to_vec()), "café");
}
