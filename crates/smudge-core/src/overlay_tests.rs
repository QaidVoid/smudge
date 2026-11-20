use crate::geometry::{Point, Rect, Size};

use super::Placement;

/// The window the placements are computed in.
const WINDOW: Size = Size {
    width: 200.0,
    height: 100.0,
};

/// An overlay's size.
const POPUP: Size = Size {
    width: 60.0,
    height: 40.0,
};

#[test]
fn below_opens_under_the_node_and_flips_above_without_room() {
    let high = Rect::new(10.0, 10.0, 50.0, 30.0);
    assert_eq!(
        Placement::Below.place(high, POPUP, WINDOW),
        Point { x: 10.0, y: 30.0 }
    );
    let low = Rect::new(10.0, 70.0, 50.0, 90.0);
    assert_eq!(
        Placement::Below.place(low, POPUP, WINDOW),
        Point { x: 10.0, y: 30.0 },
        "above the node"
    );
}

#[test]
fn after_opens_right_of_the_node_and_flips_left_at_the_edge() {
    let left = Rect::new(10.0, 10.0, 50.0, 30.0);
    assert_eq!(
        Placement::After.place(left, POPUP, WINDOW),
        Point { x: 50.0, y: 10.0 }
    );
    let right = Rect::new(120.0, 10.0, 160.0, 30.0);
    assert_eq!(
        Placement::After.place(right, POPUP, WINDOW),
        Point { x: 60.0, y: 10.0 }
    );
}

#[test]
fn a_point_opens_down_and_right_and_turns_where_the_window_ends() {
    let node = Rect::new(0.0, 0.0, 200.0, 100.0);
    assert_eq!(
        Placement::At(Point { x: 20.0, y: 20.0 }).place(node, POPUP, WINDOW),
        Point { x: 20.0, y: 20.0 }
    );
    assert_eq!(
        Placement::At(Point { x: 180.0, y: 90.0 }).place(node, POPUP, WINDOW),
        Point { x: 120.0, y: 50.0 },
        "up and to the left of the point"
    );
}

#[test]
fn an_overlay_shifts_along_to_stay_inside_the_window() {
    let wide = Size {
        width: 120.0,
        height: 40.0,
    };
    let node = Rect::new(150.0, 10.0, 190.0, 30.0);
    assert_eq!(
        Placement::Below.place(node, wide, WINDOW),
        Point { x: 80.0, y: 30.0 }
    );
    let tall = Size {
        width: 60.0,
        height: 140.0,
    };
    assert_eq!(
        Placement::Below.place(node, tall, WINDOW).y,
        0.0,
        "an overlay taller than the window starts at its top"
    );
}
