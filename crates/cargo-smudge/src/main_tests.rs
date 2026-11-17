use super::{driver_command, subcommand};

#[test]
fn accepts_the_name_cargo_prefixes() {
    let smudge = String::from("smudge");
    let dev = String::from("dev");
    let single = [dev.clone()];
    assert!(driver_command(subcommand(&single)).is_some());
    assert!(driver_command(subcommand(&[smudge, dev])).is_some());
}

#[test]
fn refuses_other_commands() {
    let build = String::from("build");
    assert!(driver_command(&[build]).is_none());
}
