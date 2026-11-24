//! Tests for the URI list of a file manager and the paths it yields.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use super::{READ_LIMIT, paths_from_uri_list, read_to_end};

#[test]
fn a_plain_list_yields_the_paths_of_its_file_uris() {
    let list = "file:///tmp/a.txt\r\nfile:///home/user/b c.txt\r\n";

    assert_eq!(
        paths_from_uri_list(list),
        ["/tmp/a.txt", "/home/user/b c.txt"].map(PathBuf::from)
    );
}

#[test]
fn percent_escapes_decode_into_the_paths_they_name() {
    let list = "file:///home/user/My%20File.txt\r\nfile:///home/user/%C3%A9.txt\r\n";

    assert_eq!(
        paths_from_uri_list(list),
        ["/home/user/My File.txt", "/home/user/é.txt"].map(PathBuf::from)
    );
}

#[test]
fn a_localhost_host_names_a_file_of_this_machine() {
    let list = "file://localhost/tmp/a.txt\r\n";

    assert_eq!(paths_from_uri_list(list), [PathBuf::from("/tmp/a.txt")]);
}

#[test]
fn lines_that_are_no_local_file_yield_nothing() {
    let list = concat!(
        "# a comment\r\n",
        "\r\n",
        "https://smudge.rs/index.html\r\n",
        "file://otherhost/tmp/a.txt\r\n",
        "not a uri at all\r\n",
    );

    assert_eq!(paths_from_uri_list(list), Vec::<PathBuf>::new());
}

#[test]
fn an_authority_that_only_starts_like_localhost_yields_nothing() {
    for uri in [
        "file://localhost:8080/tmp/a.txt",
        "file://localhostfoo/tmp/a.txt",
        "file://localhost",
    ] {
        assert!(paths_from_uri_list(uri).is_empty(), "{uri}");
    }
}

#[test]
fn a_pipe_that_ends_yields_what_it_carried() {
    let Ok((reader, mut writer)) = std::io::pipe() else {
        panic!("a pipe");
    };
    let sender = std::thread::spawn(move || writer.write_all(b"file:///tmp/a.txt\r\n").is_ok());

    let received = read_to_end(reader, Duration::from_secs(1));

    assert!(sender.join().expect("the sender"));
    assert_eq!(received, Some(b"file:///tmp/a.txt\r\n".to_vec()));
}

#[test]
fn a_pipe_over_the_limit_yields_nothing() {
    let Ok((reader, mut writer)) = std::io::pipe() else {
        panic!("a pipe");
    };
    let payload = vec![b'x'; READ_LIMIT + 1];
    // A source that streams past the limit loses its pipe when the read
    // gives up, so the sender's outcome says nothing here.
    let sender = std::thread::spawn(move || {
        let _ = writer.write_all(&payload);
    });

    let received = read_to_end(reader, Duration::from_secs(5));

    sender.join().expect("the sender");
    assert_eq!(received, None);
}

#[test]
fn an_escape_that_is_not_one_stays_as_it_is() {
    let list = "file:///tmp/100%.txt\r\n";

    assert_eq!(paths_from_uri_list(list), [PathBuf::from("/tmp/100%.txt")]);
}
