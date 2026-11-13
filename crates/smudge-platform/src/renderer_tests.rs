//! Tests for the default renderer choice and its environment override.

use super::{RendererChoice, env_choice};

#[test]
fn an_unset_or_empty_variable_leaves_the_choice_alone() {
    assert_eq!(env_choice(None), Ok(None));
    assert_eq!(env_choice(Some("")), Ok(None));
}

#[test]
fn the_three_values_pick_their_renderer() {
    assert_eq!(env_choice(Some("cpu")), Ok(Some(RendererChoice::Cpu)));
    assert_eq!(env_choice(Some("gpu")), Ok(Some(RendererChoice::Gpu)));
    assert_eq!(env_choice(Some("auto")), Ok(Some(RendererChoice::Auto)));
}

#[test]
fn any_other_value_fails_with_the_value_reported() {
    assert_eq!(env_choice(Some("CUDA")), Err(String::from("CUDA")));
    assert_eq!(env_choice(Some("Auto")), Err(String::from("Auto")));
}

#[test]
fn a_window_starts_on_the_cpu_renderer_unless_it_asks_for_the_gpu() {
    assert_eq!(
        crate::window::WindowOptions::default().renderer,
        RendererChoice::Cpu
    );
}
