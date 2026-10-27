//! Tests for the environment override of the renderer choice.

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
