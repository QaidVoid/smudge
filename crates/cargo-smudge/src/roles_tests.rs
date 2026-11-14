use super::*;

#[test]
fn names_the_lto_the_link_answers_to() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    assert!(names_lto(&args(&["rustc", "-Clto", "x.rs"])));
    assert!(names_lto(&args(&["rustc", "-Clto=thin", "x.rs"])));
    assert!(names_lto(&args(&["rustc", "-Clto=yes", "x.rs"])));
    assert!(names_lto(&args(&["rustc", "-Clto=y", "x.rs"])));
    assert!(names_lto(&args(&["rustc", "-C", "lto=fat", "x.rs"])));
    // The last occurrence of an option decides, in either
    // direction.
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clto=thin",
        "-Clto=off",
        "x.rs"
    ])));
    assert!(names_lto(&args(&[
        "rustc",
        "-Clto=off",
        "-Clto=thin",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc", "-C", "lto=fat", "-C", "lto=no", "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=yes",
        "-Clinker-plugin-lto=no",
        "x.rs"
    ])));
    assert!(names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=no",
        "-Clinker-plugin-lto=yes",
        "x.rs"
    ])));
    assert!(names_lto(&args(&["rustc", "-Clinker-plugin-lto", "x.rs"])));
    assert!(names_lto(&args(&["rustc", "--codegen=lto=thin", "x.rs"])));
    assert!(names_lto(&args(&[
        "rustc",
        "--codegen",
        "lto=thin",
        "x.rs"
    ])));
    assert!(names_lto(&args(&["rustc", "--codegen", "lto", "x.rs"])));
    assert!(names_lto(&args(&[
        "rustc",
        "--codegen",
        "linker-plugin-lto",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&["rustc", "-Clto=off", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "-Clto=false", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "-Clto=no", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "-Clto=n", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "-C", "lto=off", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "--codegen", "lto=no", "x.rs"])));
    assert!(!names_lto(&args(&[
        "rustc",
        "--codegen",
        "lto=off",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=off",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=false",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=no",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=n",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "--error-format=json",
        "-Cdebuginfo=2",
        "-C",
        "extra-filename=-abc",
        "x.rs"
    ])));
}

#[test]
fn reads_the_spellings_rustc_reads() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    // Rustc reads an underscore where the flags spell a hyphen.
    assert!(names_lto(&args(&[
        "rustc",
        "-Clinker_plugin_lto=yes",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker_plugin_lto=no",
        "x.rs"
    ])));
    assert!(names_lto(&args(&["rustc", "-Clinker_plugin_lto", "x.rs"])));
    // The last occurrence decides across the spellings.
    assert!(!names_lto(&args(&[
        "rustc",
        "-Clinker-plugin-lto=yes",
        "-Clinker_plugin_lto=no",
        "x.rs"
    ])));
    assert!(names_lto(&args(&[
        "rustc",
        "-Clinker_plugin_lto=no",
        "-Clinker-plugin-lto=yes",
        "x.rs"
    ])));
    assert!(names_lto(&args(&[
        "rustc",
        "--codegen=linker_plugin_lto=yes",
        "x.rs"
    ])));
    assert!(names_lto(&args(&[
        "rustc",
        "--codegen",
        "linker_plugin_lto=yes",
        "x.rs"
    ])));
}

#[test]
fn waits_for_the_final_split_debuginfo() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-Csplit-debuginfo=unpacked",
        "x.rs"
    ])));
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-Csplit_debuginfo=unpacked",
        "x.rs"
    ])));
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "--codegen=split-debuginfo=packed",
        "x.rs"
    ])));
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-Csplit-debuginfo",
        "x.rs"
    ])));
    // The final value decides, in either direction.
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "-Csplit-debuginfo=unpacked",
        "-Csplit_debuginfo=off",
        "x.rs"
    ])));
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-Csplit_debuginfo=off",
        "-Csplit-debuginfo=unpacked",
        "x.rs"
    ])));
    // An option never named, or one another argument merely
    // spells, answers for nothing.
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "-Cdebuginfo=2",
        "x.rs"
    ])));
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "--remap-path-prefix=x=split-debuginfo=unpacked",
        "x.rs"
    ])));
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "-Clink-arg=split-debuginfo=unpacked",
        "x.rs"
    ])));
}

#[test]
fn waits_for_the_final_linker() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    let self_path = "/bin/cargo-smudge";
    // The last spelling of the option decides.
    assert_eq!(
        linker_of(
            &args(&["rustc", "-C", "linker=old-cc", "-Clinker=new-cc", "x.rs"]),
            self_path
        ),
        "new-cc"
    );
    assert_eq!(
        linker_of(
            &args(&["rustc", "-Clinker=old-cc", "-C", "linker=new-cc", "x.rs"]),
            self_path
        ),
        "new-cc"
    );
    assert_eq!(
        linker_of(
            &args(&["rustc", "--codegen=linker=long-cc", "x.rs"]),
            self_path
        ),
        "long-cc"
    );
    assert_eq!(
        linker_of(
            &args(&["rustc", "--codegen", "linker=long-cc", "x.rs"]),
            self_path
        ),
        "long-cc"
    );
    // The wrapper's own path is not a linker, and an empty value
    // names none.
    assert_eq!(
        linker_of(
            &args(&["rustc", "-Clinker=/bin/cargo-smudge", "x.rs"]),
            self_path
        ),
        "cc"
    );
    assert_eq!(
        linker_of(&args(&["rustc", "-Clinker=", "x.rs"]), self_path),
        "cc"
    );
    assert_eq!(
        linker_of(&args(&["rustc", "-Clto=thin", "x.rs"]), self_path),
        "cc"
    );
}

#[test]
fn reads_clustered_codegen_options() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    // The letters that take no value cluster, and a `C` inside the
    // cluster carries the codegen option in the same token.
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-gCsplit_debuginfo=unpacked",
        "x.rs"
    ])));
    assert!(splits_debuginfo(&args(&[
        "rustc",
        "-wgCsplit-debuginfo=unpacked",
        "x.rs"
    ])));
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "-gCsplit_debuginfo=off",
        "x.rs"
    ])));
    assert!(names_lto(&args(&["rustc", "-wClto=thin", "x.rs"])));
    assert!(!names_lto(&args(&["rustc", "-wClto=off", "x.rs"])));
}

#[test]
fn skips_another_options_operand() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    };
    // The token after an option that takes one is its operand,
    // whatever it spells.
    assert!(!splits_debuginfo(&args(&[
        "rustc",
        "--remap-path-prefix",
        "-Csplit-debuginfo=unpacked=/mapped",
        "x.rs"
    ])));
    assert!(!names_lto(&args(&["rustc", "--cfg", "x=-Clto", "x.rs"])));
    assert!(names_lto(&args(&[
        "rustc",
        "--remap-path-prefix",
        "a=b",
        "-Clto",
        "x.rs"
    ])));
    // An operand that spells a codegen option answers for nothing,
    // so the last real occurrence stands.
    assert!(names_lto(&args(&[
        "rustc",
        "-Clto",
        "--remap-path-prefix",
        "-Clto=off",
        "x.rs"
    ])));
}

#[test]
fn masks_a_clustered_operand_taker() {
    // A cluster that ends in an option whose value rides the next
    // argument spends that argument on itself, so the option it
    // spells answers for nothing, whatever gate reads it.
    let args = vec![
        String::from("rustc"),
        String::from("-gL"),
        String::from("-Clto"),
        String::from("-gwD"),
        String::from("--crate-type=proc-macro"),
    ];
    assert!(!names_lto(&args));
    assert!(!splits_debuginfo(&args));
    let mask = operand_mask(&args);
    assert!(mask[2], "the bare -gL does not spend the -Clto beside it");
    assert!(mask[4], "the lint cluster does not spend the crate type");
    // A value spelled into the token leaves the next argument
    // alone, which stays its own option.
    let attached = vec![
        String::from("rustc"),
        String::from("-gLdir"),
        String::from("-Clto=off"),
    ];
    let mask = operand_mask(&attached);
    assert!(!mask[2]);
    assert!(!names_lto(&attached));
}
