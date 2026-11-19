use super::*;

/// Fails to compile unless `T` is what a dialog library takes as a parent.
fn parent_of_a_dialog<T: HasWindowHandle + HasDisplayHandle>() {}

#[test]
fn the_handles_serve_as_the_parent_of_a_dialog() {
    parent_of_a_dialog::<WindowHandles>();
}

#[test]
fn no_open_window_answers_that_the_handles_are_unavailable() {
    let mut lent = false;
    let answer = with_window_handles(|_| lent = true);
    assert!(matches!(answer, Err(HandleError::Unavailable)));
    assert!(!lent, "nothing is lent without a window");
}
