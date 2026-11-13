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
///
/// The default is [`RendererChoice::Cpu`], with or without the `gpu`
/// feature. Creating a GPU device costs an ordinary app most of its startup
/// and memory: on a desktop with a GTX 1060, a log viewer took 720 to 1,010
/// ms of CPU to its first frame and 192 to 195 MB of peak memory on the GPU
/// renderer, against 10 to 20 ms and 28 MB on the CPU renderer. An app that
/// redraws most of a large window every frame, where the GPU renderer is
/// several times faster, asks for it with [`RendererChoice::Auto`] or
/// [`RendererChoice::Gpu`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RendererChoice {
    /// The CPU renderer. Always available, and the default.
    #[default]
    Cpu,
    /// The GPU renderer through vello.
    ///
    /// Needs the `gpu` feature and a Vulkan, Metal or DX12 adapter that can
    /// present to the window. A run with this choice fails when there is
    /// none, and it never falls back to the CPU renderer on its own. A
    /// machine whose only GPU driver is OpenGL has no such adapter, because
    /// the OpenGL backend is not compiled in.
    Gpu,
    /// The GPU renderer when the window's adapter is a real GPU, and the CPU
    /// renderer otherwise: when the adapter is a software rasterizer such as
    /// llvmpipe or SwiftShader, when no Vulkan, Metal or DX12 adapter exists,
    /// when the window surface cannot present GPU frames, while the damage
    /// overlay is on, or in a build without the `gpu` feature.
    ///
    /// This is how an app that wants the GPU renderer where it runs well asks
    /// for it without failing elsewhere.
    Auto,
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
