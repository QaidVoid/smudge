//! Tests for the backends the renderer is built with.

use vello::wgpu::{Backends, Instance};

#[test]
fn only_the_primary_backends_are_compiled() {
    let compiled = Instance::enabled_backend_features();
    assert!(
        !compiled.intersects(Backends::GL),
        "a dependency compiled the OpenGL backend back in: {compiled:?}"
    );
    assert!(Backends::PRIMARY.contains(compiled), "{compiled:?}");
}
