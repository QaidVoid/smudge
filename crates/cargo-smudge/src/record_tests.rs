use super::*;

#[test]
fn names_the_crate_an_artifact_carries() {
    assert_eq!(
        crate_of_artifact("/store/deps/libui-fab4ef1950f375be.rlib").as_deref(),
        Some("ui")
    );
    assert_eq!(
        crate_of_artifact("/store/deps/libcore2-c132cc7bceb6a2f1.rmeta").as_deref(),
        Some("core2")
    );
    assert_eq!(
        crate_of_artifact("/store/deps/libmac-565532a9e67c1f5e.so").as_deref(),
        Some("mac")
    );
    assert_eq!(crate_of_stem("ui-fab4ef1950f375be").as_deref(), Some("ui"));
    assert_eq!(crate_of_stem("plain").as_deref(), Some("plain"));
}

#[test]
fn reads_dep_info_dependencies() {
    let dir = std::env::temp_dir().join("smudge-dep-info-test");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    // Rustc escapes a space and leaves every other byte literal, and
    // keeps the parent components a stray `#[path]` produces. A name
    // that carries a literal backslash before its space writes two
    // backslashes in a row, which only the first one's neighbors can
    // tell apart.
    let rule = "out/ui-abc.d: ui/src/lib.rs ui/src/sub\\ dir/mod\\ with\\ \
         space.rs ui/src/back\\slash.rs ui/src/back\\\\ space.rs \
         ui/src/nested/../label.rs ui/src/label\ttag.rs ui/src/end\t \
         ui/link/../content.txt";
    let path = dir.join("ui-abc.d");
    fs::write(&path, format!("{rule}\n\nui/src/lib.rs:\n")).expect("the rule could not be written");
    let sources = dep_info_sources(&path, Path::new("/ws"));
    // The raw spellings stay beside the normalized ones, so a link
    // the name carries is still watched. A backslash before a
    // separating space reads two ways, one name or two, and a
    // section of several pairs reads plain up to any pair and
    // escaped past it, so every reading joins the learned names.
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/dir/mod with space.rs"),
            PathBuf::from("/ws/dir/mod\\"),
            PathBuf::from("/ws/space.rs"),
            PathBuf::from("/ws/ui/content.txt"),
            PathBuf::from("/ws/ui/link/../content.txt"),
            PathBuf::from("/ws/ui/src/back\\ space.rs"),
            PathBuf::from("/ws/ui/src/back\\\\"),
            PathBuf::from("/ws/ui/src/back\\slash.rs"),
            PathBuf::from("/ws/ui/src/end\t"),
            PathBuf::from("/ws/ui/src/label\ttag.rs"),
            PathBuf::from("/ws/ui/src/label.rs"),
            PathBuf::from("/ws/ui/src/lib.rs"),
            PathBuf::from("/ws/ui/src/nested/../label.rs"),
            PathBuf::from("/ws/ui/src/sub dir/mod with space.rs"),
            PathBuf::from("/ws/ui/src/sub\\"),
            PathBuf::from("/ws/with space.rs"),
            PathBuf::from("/ws/with\\"),
        ]
    );
}

#[test]
fn keeps_every_byte_of_a_name_without_escapes() {
    let dir = std::env::temp_dir().join("smudge-dep-info-control");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let path = dir.join("ui-abc.d");
    fs::write(&path, "out/ui-abc.d: ui/src/lib.rs data/payload\u{1}.txt\n")
        .expect("the rule could not be written");
    assert_eq!(
        dep_info_sources(&path, Path::new("/ws")),
        vec![
            PathBuf::from("/ws/data/payload\u{1}.txt"),
            PathBuf::from("/ws/ui/src/lib.rs"),
        ]
    );
}

#[test]
fn reads_names_past_the_cap_of_escaped_spaces() {
    let dir = std::env::temp_dir().join("smudge-dep-info-many");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let path = dir.join("ui-abc.d");
    let spaced = vec!["a"; 22].join("\\ ");
    fs::write(&path, format!("out/ui-abc.d: {spaced}.rs ui/src/lib.rs\n"))
        .expect("the rule could not be written");
    let sources = dep_info_sources(&path, Path::new("/ws"));
    // Past the cap, the escaped reading still ends a name at a
    // plain space.
    let joined = PathBuf::from(format!("/ws/{}.rs", vec!["a"; 22].join(" ")));
    assert!(sources.contains(&joined), "{sources:?}");
    assert!(
        sources.contains(&PathBuf::from("/ws/ui/src/lib.rs")),
        "{sources:?}"
    );
}

#[test]
fn reads_diagnostic_span_sources() {
    let human = "error[E0758]: unterminated block comment\n   \
         --> ui/src/../late_out.rs:4:1\n    |\n    | /* never closed\n    |   ^\n\nerror: could not compile `ui` due to 1 previous error\n";
    let sources = span_sources(human, Path::new("/ws"), &[]);
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/late_out.rs"),
            PathBuf::from("/ws/ui/src/../late_out.rs"),
        ]
    );
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"ui/src/../late_out.rs\",\
         \"line_start\":3}],\"rendered\":\"...--> ui/src/../late_out.rs:3:1...\"}\n";
    let sources = span_sources(json, Path::new("/ws"), &[]);
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/late_out.rs"),
            PathBuf::from("/ws/ui/src/../late_out.rs"),
        ]
    );
}

#[test]
fn reads_the_source_a_span_names_whatever_extension() {
    // A module a `#[path]` attribute names can carry any extension,
    // and a failed compile's spans are the only place it appears
    // before dep-info exists. A real file of any extension is
    // learned; the compiler's own synthetic paths, which name
    // nothing on disk, stay out.
    let base = std::env::temp_dir().join("smudge-span-ext");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ui").join("src")).expect("the scratch could not be made");
    fs::write(base.join("ui").join("late.inc"), "/* never closed\n")
        .expect("the module could not be written");
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"ui/src/../late.inc\"}]}\n";
    let sources = span_sources(json, &base, &[]);
    assert_eq!(
        sources,
        vec![
            base.join("ui").join("late.inc"),
            base.join("ui").join("src").join("..").join("late.inc"),
        ]
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn reads_the_input_a_file_read_error_names() {
    // A read that fails names the input it could not open, which has
    // no `.rs` extension to survive a filter and no span of its own:
    // the message is what teaches the session a file to watch, so
    // creating it retries the crate that asked for it.
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"couldn't read \
         `ui/src/../content.txt`: No such file or directory (os error 2)\",\
         \"code\":null,\"level\":\"error\",\"spans\":[{\"file_name\":\"ui/src/lib.rs\"}]}\n";
    let sources = span_sources(json, Path::new("/ws"), &[]);
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/content.txt"),
            PathBuf::from("/ws/ui/src/../content.txt"),
            PathBuf::from("/ws/ui/src/lib.rs"),
        ]
    );
    let human = "error: couldn't read ui/src/../content.txt: No such file or \
         directory (os error 2)\n --> ui/src/lib.rs:6:5\n";
    let sources = span_sources(human, Path::new("/ws"), &[]);
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/content.txt"),
            PathBuf::from("/ws/ui/src/../content.txt"),
            PathBuf::from("/ws/ui/src/lib.rs"),
        ]
    );
    // Rustc writes a backtick of the name literally, so the closing
    // delimiter is the last one the line carries and the whole name
    // is learned, whose creation retries the crate.
    let backticked = "error: couldn't read `ui/src/data/a`b.txt`: No such \
         file or directory (os error 2)\n";
    let sources = span_sources(backticked, Path::new("/ws"), &[]);
    assert_eq!(sources, vec![PathBuf::from("/ws/ui/src/data/a`b.txt")]);
}

#[test]
fn skips_another_options_operand_when_reading_values() {
    // A remapping operand that spells an option answers for nothing;
    // the crate types a record carries are the ones rustc reads.
    let args = vec![
        String::from("rustc"),
        String::from("--crate-name"),
        String::from("ui"),
        String::from("--remap-path-prefix"),
        String::from("--crate-type=proc-macro"),
        String::from("--crate-type"),
        String::from("lib"),
        String::from("-Cextra-filename=-abc"),
        String::from("--remap-path-prefix"),
        String::from("-Cextra-filename=remapped"),
    ];
    assert_eq!(
        values_in(&args, "--crate-type").collect::<Vec<_>>(),
        ["lib"]
    );
    assert_eq!(artifact_stem_of_args(&args).as_deref(), Some("ui-abc"));
}

#[test]
fn reads_an_artifact_stem_from_arguments() {
    let args = vec![
        String::from("rustc"),
        String::from("--crate-name"),
        String::from("ui"),
        String::from("-C"),
        String::from("extra-filename=-abc123"),
    ];
    assert_eq!(artifact_stem_of_args(&args).as_deref(), Some("ui-abc123"));
    let joined = vec![
        String::from("rustc"),
        String::from("--crate-name=ui"),
        String::from("-Cextra-filename=-abc123"),
    ];
    assert_eq!(artifact_stem_of_args(&joined).as_deref(), Some("ui-abc123"));
}

#[test]
fn unremaps_a_diagnostic_name_the_remap_spelled() {
    // A recorded remapping spells the names the diagnostics carry:
    // the spelling the compile read joins the candidates, so
    // repairing the real file answers even when the remapped one
    // names nothing.
    let base = std::env::temp_dir().join("smudge-remap-span");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ui").join("src")).expect("the scratch could not be made");
    fs::write(base.join("ui").join("late_out.rs"), "/* unterminated\n")
        .expect("the module could not be written");
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"remapped-ui/src/../late_out.rs\"}]}\n";
    let remaps = vec![(String::from("ui"), String::from("remapped-ui"))];
    let sources = span_sources(json, &base, &remaps);
    assert!(
        sources.contains(&base.join("ui").join("late_out.rs")),
        "the physical spelling the compile read was not learned:\n{sources:?}"
    );
    let args = vec![
        String::from("rustc"),
        String::from("--remap-path-prefix=ui=remapped-ui"),
        String::from("--remap-path-prefix"),
        String::from("engine=remapped-engine"),
    ];
    assert_eq!(
        unremap_of(&args),
        vec![
            (String::from("ui"), String::from("remapped-ui")),
            (String::from("engine"), String::from("remapped-engine")),
        ]
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn splits_a_remapping_pair_at_the_last_equals() {
    // Rustc splits a pair at the last `=`, so the `from` side can
    // carry the character; the reader must answer the same split
    // or the physical name never recovers.
    let args = vec![
        String::from("rustc"),
        String::from("--remap-path-prefix=ui=physical=virtual-ui"),
        String::from("--remap-path-prefix"),
        String::from("engine=phys=ical"),
    ];
    assert_eq!(
        unremap_of(&args),
        vec![
            (String::from("ui=physical"), String::from("virtual-ui")),
            (String::from("engine=phys"), String::from("ical")),
        ]
    );
}

#[test]
fn unremaps_a_spelling_whose_replacement_ends_in_a_separator() {
    // A replacement that ends in a separator joins the remainder
    // without another one, so the boundary check that demands a
    // leading slash loses the name entirely.
    let base = std::env::temp_dir().join("smudge-remap-sep");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ui").join("src")).expect("the scratch could not be made");
    fs::write(base.join("ui").join("late.inc"), "/* unterminated\n")
        .expect("the module could not be written");
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"virtual-ui/src/../late.inc\"}]}\n";
    let remaps = vec![(String::from("ui"), String::from("virtual-ui/"))];
    let sources = span_sources(json, &base, &remaps);
    assert!(
        sources.contains(&base.join("ui").join("src").join("..").join("late.inc")),
        "the physical spelling under the separated replacement was not learned:\n{sources:?}"
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn keeps_a_remapped_span_whose_physical_module_is_not_rust() {
    // A remapped spelling of a module whose extension is not `rs`
    // names no file, so the existence of the physical spelling it
    // recovers is what keeps the pair learned.
    let base = std::env::temp_dir().join("smudge-remap-ext");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ui").join("src")).expect("the scratch could not be made");
    fs::write(base.join("ui").join("late.inc"), "/* unterminated\n")
        .expect("the module could not be written");
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"virtual-ui/src/../late.inc\"}]}\n";
    let remaps = vec![(String::from("ui"), String::from("virtual-ui"))];
    let sources = span_sources(json, &base, &remaps);
    assert!(
        sources.contains(&base.join("ui").join("src").join("..").join("late.inc")),
        "the physical spelling of the non-Rust module was not learned:\n{sources:?}"
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn unmaps_a_span_an_empty_replacement_removed() {
    // A pair whose replacement is empty strips the prefix and its
    // separator, so the physical spelling joins the remainder with
    // one slash of its own; the pair whose source is empty maps
    // nothing, which rustc skips the same way.
    let base = std::env::temp_dir().join("smudge-remap-empty");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ui").join("src")).expect("the scratch could not be made");
    fs::write(base.join("ui").join("late.inc"), "/* unterminated\n")
        .expect("the module could not be written");
    let json = "{\"$message_type\":\"diagnostic\",\"message\":\"unterminated \
         block comment\",\"spans\":[{\"file_name\":\"src/../late.inc\"}]}\n";
    let remaps = vec![(String::from("ui"), String::new())];
    let sources = span_sources(json, &base, &remaps);
    assert!(
        sources.contains(&base.join("ui").join("src").join("..").join("late.inc")),
        "the physical spelling under the empty replacement was not learned:\n{sources:?}"
    );
    let args = vec![
        String::from("rustc"),
        String::from("--remap-path-prefix==virtual"),
    ];
    assert_eq!(unremap_of(&args), Vec::new());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn reads_a_dependency_whose_name_carries_a_newline() {
    // Rustc writes a newline of a file name literally between its
    // bytes, so the rule ends at the blank line that separates it
    // from the next one, and the newline belongs to the name.
    let dir = std::env::temp_dir().join("smudge-dep-info-newline");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let rule = "out/ui.d: ui/src/lib.rs ui/src/../li\
         ne\nname.inc";
    let path = dir.join("ui.d");
    fs::write(&path, format!("{rule}\n\nui/src/lib.rs:\n")).expect("the rule could not be written");
    let sources = dep_info_sources(&path, Path::new("/ws"));
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/line\nname.inc"),
            PathBuf::from("/ws/ui/src/../line\nname.inc"),
            PathBuf::from("/ws/ui/src/lib.rs"),
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reads_a_dependency_whose_name_carries_a_carriage_return() {
    // A carriage return inside a file name is a byte of the name:
    // the lines of the rule split on the newline alone, so the
    // name is learned whole and its edits are seen.
    let dir = std::env::temp_dir().join("smudge-dep-info-return");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let rule = "out/ui.d: ui/src/lib.rs ui/na\rme.inc";
    let path = dir.join("ui.d");
    fs::write(&path, format!("{rule}\n\n")).expect("the rule could not be written");
    let sources = dep_info_sources(&path, Path::new("/ws"));
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/na\rme.inc"),
            PathBuf::from("/ws/ui/src/lib.rs"),
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reads_a_backslash_space_rule_both_ways() {
    // Rustc escapes a space in a name with a backslash but writes
    // a backslash in a name plain, so `a\ z.txt` names either one
    // file or two; both readings are learned, and the watches
    // answer whichever the file system carries.
    let dir = std::env::temp_dir().join("smudge-dep-info-backslash");
    fs::create_dir_all(&dir).expect("the scratch directory could not be made");
    let rule = "out/ui.d: ui/src/lib.rs ui/a\\ z.txt";
    let path = dir.join("ui.d");
    fs::write(&path, format!("{rule}\n\n")).expect("the rule could not be written");
    let sources = dep_info_sources(&path, Path::new("/ws"));
    assert_eq!(
        sources,
        vec![
            PathBuf::from("/ws/ui/a z.txt"),
            PathBuf::from("/ws/ui/a\\"),
            PathBuf::from("/ws/ui/src/lib.rs"),
            PathBuf::from("/ws/z.txt"),
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Arguments of a rustc invocation cargo makes for `name` with the given
/// crate type, out directory and extra file name.
fn invocation(name: &str, kind: &str, out_dir: &str, extra: Option<&str>) -> Vec<String> {
    let mut args = vec![
        String::from("--crate-name"),
        String::from(name),
        String::from("--crate-type"),
        String::from(kind),
        String::from("--out-dir"),
        String::from(out_dir),
    ];
    if let Some(extra) = extra {
        args.push(String::from("-C"));
        args.push(format!("extra-filename={extra}"));
    }
    args
}

#[test]
fn reads_a_unit_s_place_in_either_build_layout() {
    let unit = Path::new("/t/debug/build/core/7c66c3d37d63087f/out");
    assert_eq!(
        UnitDir::of(unit),
        Some(UnitDir::Unit {
            build: Path::new("/t/debug/build"),
            package: "core",
            hash: "7c66c3d37d63087f",
        })
    );
    let script = Path::new("/t/debug/build/probe-20d6ec51a575fc55");
    assert_eq!(
        UnitDir::of(script),
        Some(UnitDir::FlatScript {
            build: Path::new("/t/debug/build"),
            package: "probe",
        })
    );
    let deps = Path::new("/t/debug/deps");
    assert_eq!(
        UnitDir::of(deps),
        Some(UnitDir::Flat {
            profile: Path::new("/t/debug")
        })
    );
    for dir in [unit, script, deps] {
        assert_eq!(
            UnitDir::of(dir).map(|unit| unit.build_dir()),
            Some(PathBuf::from("/t/debug/build")),
            "{dir:?} belongs to the profile's build directory"
        );
    }
    assert!(
        !UnitDir::of(Path::new("/t/debug/build/out")).is_some_and(|unit| unit.holds_a_script()),
        "an `out` without a package and a hash above it is no unit's directory"
    );
}

#[test]
fn tells_a_build_script_from_a_binary_named_like_one() {
    let script = invocation(
        "build_script_build",
        "bin",
        "/t/debug/build/probe/e6975c4796e28a84/out",
        None,
    );
    let flat_script = invocation(
        "build_script_build",
        "bin",
        "/t/debug/build/probe-20d6ec51a575fc55",
        Some("-20d6ec51a575fc55"),
    );
    assert!(compiles_a_script(&script, None));
    assert!(compiles_a_script(&flat_script, None));

    let bin = invocation(
        "build_script_extra",
        "bin",
        "/t/debug/build/probe/b850606bdaa55b00/out",
        None,
    );
    assert!(
        !compiles_a_script(&bin, Some("build_script_extra")),
        "cargo names a binary target in CARGO_BIN_NAME and a script never"
    );
    let flat_bin = invocation(
        "build_script_extra",
        "bin",
        "/t/debug/deps",
        Some("-b850606bdaa55b00"),
    );
    assert!(!compiles_a_script(&flat_bin, None));
    let lib = invocation(
        "build_script_util",
        "lib",
        "/t/debug/build/probe/aa11/out",
        Some("-aa11"),
    );
    assert!(!compiles_a_script(&lib, None));
}

#[test]
fn files_each_package_s_script_under_a_stem_of_its_own() {
    let first = invocation(
        "build_script_build",
        "bin",
        "/t/debug/build/core/1441384070ef7c7c/out",
        None,
    );
    let second = invocation(
        "build_script_build",
        "bin",
        "/t/debug/build/ui/6e819f9ce2d28dc3/out",
        None,
    );
    assert_eq!(
        record_stem_of_args(&first).as_deref(),
        Some("build_script_build-1441384070ef7c7c")
    );
    assert_eq!(
        record_stem_of_args(&second).as_deref(),
        Some("build_script_build-6e819f9ce2d28dc3")
    );

    let lib = invocation(
        "core",
        "lib",
        "/t/debug/build/core/7c66c3d37d63087f/out",
        Some("-7c66c3d37d63087f"),
    );
    assert_eq!(
        record_stem_of_args(&lib).as_deref(),
        Some("core-7c66c3d37d63087f"),
        "a stem that carries the unit's hash keeps it once"
    );
    let flat = invocation(
        "build_script_build",
        "bin",
        "/t/debug/build/core-20d6ec51a575fc55",
        Some("-20d6ec51a575fc55"),
    );
    assert_eq!(
        record_stem_of_args(&flat).as_deref(),
        Some("build_script_build-20d6ec51a575fc55")
    );
}
