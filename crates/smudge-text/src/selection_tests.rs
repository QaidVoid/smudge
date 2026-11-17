use std::rc::Rc;

use super::{Endpoint, Order, block_range, carried, join, ordered};

/// An end in the block of `key` at `offset`.
fn end(key: u32, offset: usize) -> Endpoint<u32> {
    Endpoint { key, offset }
}

/// The order of `keys`.
fn order(keys: &[u32]) -> Order<u32> {
    Order::new(Rc::from(keys))
}

#[test]
fn ends_order_by_block_then_offset() {
    let order = order(&[3, 1, 2]);
    assert_eq!(
        ordered(&order, &end(2, 0), &end(1, 5)),
        Some((end(1, 5), end(2, 0)))
    );
    assert_eq!(
        ordered(&order, &end(1, 4), &end(1, 2)),
        Some((end(1, 2), end(1, 4)))
    );
    assert_eq!(
        ordered(&order, &end(9, 0), &end(1, 0)),
        None,
        "a key not in the list"
    );
}

#[test]
fn each_block_gets_its_part_of_the_selection() {
    let order = order(&[10, 11, 12, 13]);
    let (start, finish) = (end(11, 3), end(12, 4));
    assert_eq!(block_range(&order, &start, &finish, &10), None);
    assert_eq!(
        block_range(&order, &start, &finish, &11),
        Some(3..usize::MAX)
    );
    assert_eq!(block_range(&order, &start, &finish, &12), Some(0..4));
    assert_eq!(block_range(&order, &start, &finish, &13), None);
    assert_eq!(
        block_range(&order, &end(11, 2), &end(11, 2), &11),
        None,
        "an empty selection covers nothing"
    );
}

#[test]
fn copy_joins_blocks_in_order_with_newlines() {
    let order = order(&[0, 1, 2]);
    let texts = ["alpha beta", "gamma", "delta epsilon"];
    let joined = join(&order, &end(0, 6), &end(2, 5), |key| {
        texts[*key as usize].to_owned()
    });
    assert_eq!(joined, "beta\ngamma\ndelta");
}

#[test]
fn copy_keeps_to_character_boundaries() {
    let order = order(&[0]);
    // The offset 2 falls inside the two bytes of "é".
    let joined = join(&order, &end(0, 0), &end(0, 2), |_| String::from("aé"));
    assert_eq!(joined, "a");
}

#[test]
fn a_selection_keeps_the_blocks_that_remain_when_its_ends_leave() {
    let old = order(&[0, 1, 2, 3, 4]);
    // The first two lines are trimmed from the top, the start's among them.
    let trimmed = order(&[2, 3, 4]);
    assert_eq!(
        carried(&old, &trimmed, &end(1, 3), &end(3, 2)),
        Some((end(2, 0), end(3, 2))),
        "the start moves to the beginning of the first block that remains"
    );
    // The end's block goes from the middle.
    let without_end = order(&[0, 1, 2, 4]);
    assert_eq!(
        carried(&old, &without_end, &end(1, 3), &end(3, 2)),
        Some((end(1, 3), end(2, usize::MAX))),
        "the end moves to the end of the last block that remains"
    );
    // Every selected block goes.
    assert_eq!(carried(&old, &order(&[0, 4]), &end(1, 3), &end(3, 2)), None);
}

#[test]
fn a_block_without_a_layout_steps_through_its_text() {
    use super::{Step, step_text};

    let text = "one two\nthree";
    assert_eq!(step_text(text, 0, Step::Right), Some(1));
    assert_eq!(step_text(text, 0, Step::Left), None, "the start is an edge");
    assert_eq!(step_text(text, 0, Step::WordRight), Some(3));
    assert_eq!(step_text(text, 7, Step::WordLeft), Some(4));
    assert_eq!(
        step_text(text, 2, Step::Down),
        Some(8),
        "to the next hard line"
    );
    assert_eq!(step_text(text, 10, Step::Up), Some(7));
    assert_eq!(step_text(text, 10, Step::LineStart), Some(8));
    assert_eq!(step_text(text, 2, Step::LineEnd), Some(7));
    assert_eq!(step_text(text, 13, Step::Right), None, "the end is an edge");
    assert_eq!(
        step_text("aé", 1, Step::Right),
        Some(3),
        "a whole character"
    );
}
