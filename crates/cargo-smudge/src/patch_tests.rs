use super::names_a_closure_of;
use crate::dwarf::closure_of;

#[test]
fn finds_the_closures_of_a_function_by_its_whole_path() {
    let parents = vec![String::from("app::main")];
    assert!(names_a_closure_of(
        "cc=|()|T:app::main::{closure_env#1}#ab12",
        &parents
    ));
    assert!(names_a_closure_of(
        "cc=|()|&app::main::{closure_env#0}#ab12",
        &parents
    ));
    // Another function's closures, and a path that only ends alike.
    assert!(!names_a_closure_of(
        "cc=|()|T:app::view::{closure_env#1}#ab12",
        &parents
    ));
    assert!(!names_a_closure_of(
        "cc=|()|T:other::app::main::{closure_env#1}#ab12",
        &parents
    ));
    // A closure nested in a closure of the function moves with it.
    assert!(names_a_closure_of(
        "cc=|()|T:app::main::{closure#0}::{closure_env#0}#ab12",
        &parents
    ));
    // A closure nested in an async block of the function moves with it.
    assert!(names_a_closure_of(
        "cc=|()|T:app::main::{async_block#1}::{closure_env#0}#ab12",
        &parents
    ));
    // A path that only ends alike, without a separator before it.
    assert!(!names_a_closure_of(
        "cc=|()|T:xapp::main::{closure_env#1}#ab12",
        &parents
    ));
}

#[test]
fn reads_a_closures_function_and_number() {
    assert_eq!(
        closure_of("app::main::{closure_env#2}"),
        Some((String::from("app::main"), 2))
    );
    // The type arguments of a generic closure can name other closures.
    assert_eq!(
        closure_of("core::wrap::{closure_env#0}<app::main::{closure_env#3}>"),
        Some((String::from("core::wrap"), 0))
    );
    assert_eq!(
        closure_of("app::main::{async_block_env#1}"),
        Some((String::from("app::main"), 1))
    );
    assert_eq!(closure_of("app::main::Thing"), None);
    assert_eq!(closure_of("app::main::{impl#0}"), None);
}
