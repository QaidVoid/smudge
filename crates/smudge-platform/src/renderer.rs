//! The renderer a window presents with, and the environment override on it.

use std::env;

use crate::error::PlatformError;

/// The environment variable that overrides the renderer choice.
///
/// Its value is `cpu`, `gpu` or `auto`. Setting it lets a machine switch
/// renderer without a rebuild, which is how a check of the automatic choice
/// runs on a machine with only a software adapter.
pub const RENDERER_ENVIRONMENT: &str = "SMUDGE_RENDERER";

/// Which renderer a window presents with.
///
/// The CPU renderer keeps a pixmap and repaints only the frame's damage. The
/// GPU renderer draws the whole frame on every present, with no damage
/// tracking. Both draw the same display list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RendererChoice {
    /// The CPU renderer. Always available.
    Cpu,
    /// The GPU renderer through vello.
    ///
    /// Needs the `gpu` feature and an adapter that can present to the window.
    /// A run with this choice fails when either is missing; it never falls
    /// back on its own.
    Gpu,
    /// The GPU renderer when the window's adapter is a real GPU, and the CPU
    /// renderer otherwise: when the adapter is a software rasterizer such as
    /// llvmpipe or SwiftShader, when no adapter exists, when the window
    /// surface cannot present GPU frames, or while the damage overlay is on.
    /// The default in builds with the `gpu` feature.
    Auto,
}

impl Default for RendererChoice {
    fn default() -> RendererChoice {
        // The default depends on the feature: `Auto` with the gpu feature and
        // `Cpu` without it, which a derive cannot switch by feature.
        if cfg!(feature = "gpu") {
            RendererChoice::Auto
        } else {
            RendererChoice::Cpu
        }
    }
}

impl RendererChoice {
    /// Returns the choice with the environment override applied.
    ///
    /// [`RENDERER_ENVIRONMENT`], when it holds `cpu`, `gpu` or `auto`,
    /// replaces this choice; any other non-empty value fails the run, and an
    /// unset or empty variable leaves the choice alone.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Renderer`] when the environment variable
    /// holds an unknown value.
    pub(crate) fn with_env_override(self) -> Result<RendererChoice, PlatformError> {
        let value = env::var(RENDERER_ENVIRONMENT).ok();
        match env_choice(value.as_deref()) {
            Ok(Some(overridden)) => Ok(overridden),
            Ok(None) => Ok(self),
            Err(value) => Err(PlatformError::Renderer(format!(
                "{RENDERER_ENVIRONMENT} is set to {value:?}; expected cpu, gpu or auto"
            ))),
        }
    }
}

/// Parses one override value, or `None` when the variable is unset or empty.
fn env_choice(value: Option<&str>) -> Result<Option<RendererChoice>, String> {
    match value {
        None | Some("") => Ok(None),
        Some("cpu") => Ok(Some(RendererChoice::Cpu)),
        Some("gpu") => Ok(Some(RendererChoice::Gpu)),
        Some("auto") => Ok(Some(RendererChoice::Auto)),
        Some(other) => Err(String::from(other)),
    }
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
