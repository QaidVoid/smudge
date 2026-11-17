use super::{Kind, Scaffold, parse};

/// The arguments of one invocation.
fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

#[test]
fn passes_cargos_package_options_through() {
    assert_eq!(
        parse(&args("new demo --name counter --vcs=none --edition 2024")),
        Some(Ok(Scaffold {
            kind: Kind::New,
            path: Some(String::from("demo")),
            passed: args("--name counter --vcs none --edition 2024"),
        }))
    );
}

#[test]
fn init_defaults_to_the_current_directory() {
    assert_eq!(
        parse(&args("init")),
        Some(Ok(Scaffold {
            kind: Kind::Init,
            path: None,
            passed: Vec::new(),
        }))
    );
}

#[test]
fn refuses_other_options_and_a_missing_path() {
    assert!(matches!(parse(&args("new demo --lib")), Some(Err(_))));
    assert!(matches!(parse(&args("new")), Some(Err(_))));
    assert!(matches!(parse(&args("new a b")), Some(Err(_))));
    assert!(matches!(parse(&args("new demo --name")), Some(Err(_))));
}

#[test]
fn leaves_other_commands_alone() {
    assert_eq!(parse(&args("dev")), None);
    assert_eq!(parse(&args("build")), None);
}

#[test]
fn adds_smudge_at_this_commands_own_version_exactly() {
    // The arguments run in the package's directory, so they name no path
    // a second time.
    assert_eq!(
        super::add_arguments(env!("CARGO_PKG_VERSION")),
        vec![
            String::from("add"),
            format!("smudge@={}", env!("CARGO_PKG_VERSION")),
        ]
    );
}
