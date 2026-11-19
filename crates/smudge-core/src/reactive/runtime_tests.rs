/// The devtools promise to cost nothing with their feature off, so a field
/// they add to trace a signal or an effect must sit behind the feature. This
/// bound catches one that lands outside it. A field the node needs anyway
/// raises it on purpose.
#[cfg(target_pointer_width = "64")]
#[test]
fn a_reactive_node_keeps_its_size() {
    assert!(std::mem::size_of::<super::Node>() <= 80);
}
