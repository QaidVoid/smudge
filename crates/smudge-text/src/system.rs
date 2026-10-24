//! Per-thread ownership of parley's font and layout contexts.
//!
//! parley's contexts are not `Send`, so each thread owns one system in a
//! thread-local. Widgets reach it through [`TextSystem::with`], which creates
//! a system on first use and reuses it afterwards.
//!
//! A system created by [`TextSystem::bundled_only`] sees the embedded fonts
//! and nothing else, which is what tests and wasm build on. A system created
//! by [`TextSystem::with_system_fonts`] sees the fonts installed on the host
//! as well. A generic family such as sans serif then resolves to the host's
//! choices first and to the bundled face last, while the default text style
//! names the bundled family directly, so it never depends on the host.

use std::cell::RefCell;

/// The font and layout contexts a thread lays text out with.
///
/// One system per thread keeps parley's non-`Send` contexts out of shared
/// state. The system is inert state: creating, replacing or dropping one
/// touches nothing but this thread.
pub struct TextSystem {
    /// The registered fonts and the queries against them.
    pub(crate) fonts: parley::FontContext,
    /// Scratch space reused across layouts.
    pub(crate) layouts: parley::LayoutContext<()>,
}

impl TextSystem {
    /// Creates a system that sees only the bundled fonts.
    ///
    /// Without the `bundled-fonts` feature the system starts empty and every
    /// glyph lays out as missing.
    pub fn bundled_only() -> TextSystem {
        let collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let source_cache =
            fontique::SourceCache::new(fontique::SourceCacheOptions { shared: false });

        let mut system = TextSystem {
            fonts: parley::FontContext {
                collection,
                source_cache,
            },
            layouts: parley::LayoutContext::new(),
        };
        system.register_bundled_fonts();
        system
    }

    /// Creates a system that sees the system fonts plus the bundled ones as a
    /// fallback.
    ///
    /// System fonts are discovered only when the `system-fonts` feature is
    /// enabled, which is impossible on wasm. Without it this behaves like
    /// [`TextSystem::bundled_only`].
    pub fn with_system_fonts() -> TextSystem {
        let mut system = TextSystem {
            fonts: parley::FontContext::default(),
            layouts: parley::LayoutContext::new(),
        };
        system.register_bundled_fonts();
        system
    }

    /// Installs this system for the current thread, replacing whatever the
    /// thread owned before.
    ///
    /// # Panics
    ///
    /// Panics if the thread's system is in use, because a system cannot be
    /// replaced while a layout borrows it.
    pub fn install(self) {
        CURRENT.with(|slot| {
            let mut slot = slot.try_borrow_mut().expect(
                "the thread's text system cannot be replaced while it is in \
                 use",
            );
            *slot = Some(self);
        });
    }

    /// Runs `f` with the current thread's system, creating one on first use:
    /// [`TextSystem::with_system_fonts`] on native targets and
    /// [`TextSystem::bundled_only`] on wasm.
    ///
    /// # Panics
    ///
    /// Panics if called from inside another `with` call on the same thread,
    /// for example from a [`crate::TextLayout`] constructor, because a system
    /// cannot be handed out twice at once.
    pub fn with<R>(f: impl FnOnce(&mut TextSystem) -> R) -> R {
        CURRENT.with(|slot| {
            let mut slot = slot.try_borrow_mut().expect(
                "TextSystem::with cannot run inside another with call on the \
                 same thread",
            );
            let system = slot.get_or_insert_with(default_for_platform);
            f(system)
        })
    }

    /// Registers the bundled faces and makes each one the last choice of its
    /// generic family, so that text without an explicit family still reaches
    /// them when the host offers nothing else.
    #[cfg(feature = "bundled-fonts")]
    fn register_bundled_fonts(&mut self) {
        use std::sync::Arc;

        let register = |system: &mut TextSystem, data: &'static [u8]| {
            let blob = fontique::Blob::new(Arc::new(data));
            system.fonts.collection.register_fonts(blob, None)
        };

        let sans = register(self, crate::fonts::SANS_REGULAR);
        let sans_bold = register(self, crate::fonts::SANS_BOLD);
        let mono = register(self, crate::fonts::MONO_REGULAR);

        let sans_family = sans
            .first()
            .map(|(family, _)| *family)
            .expect("a font file yields at least one family");
        let sans_bold_family = sans_bold
            .first()
            .map(|(family, _)| *family)
            .expect("a font file yields at least one family");
        debug_assert_eq!(
            sans_family, sans_bold_family,
            "the bundled sans faces share a family name"
        );
        let mono_family = mono
            .first()
            .map(|(family, _)| *family)
            .expect("a font file yields at least one family");

        self.fall_back_to(fontique::GenericFamily::SansSerif, sans_family);
        self.fall_back_to(fontique::GenericFamily::Monospace, mono_family);
    }

    /// Puts `family` at the end of the families `generic` resolves to, unless
    /// it is already among them.
    ///
    /// The collection lists registered families ahead of the system's own
    /// choices, so appending would put the bundled face first. Rebuilding the
    /// list keeps the system's order and makes the bundled face its fallback.
    #[cfg(feature = "bundled-fonts")]
    fn fall_back_to(&mut self, generic: fontique::GenericFamily, family: fontique::FamilyId) {
        let collection = &mut self.fonts.collection;
        let mut families: Vec<_> = collection.generic_families(generic).collect();
        if !families.contains(&family) {
            families.push(family);
        }
        collection.set_generic_families(generic, families.into_iter());
    }

    /// Keeps the generic families from resolving to anything when no fonts
    /// are bundled.
    #[cfg(not(feature = "bundled-fonts"))]
    fn register_bundled_fonts(&mut self) {}
}

thread_local! {
    /// The system this thread owns, if it has one already.
    static CURRENT: RefCell<Option<TextSystem>> = const { RefCell::new(None) };
}

/// The system a thread creates when nothing was installed.
fn default_for_platform() -> TextSystem {
    #[cfg(target_arch = "wasm32")]
    {
        TextSystem::bundled_only()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        TextSystem::with_system_fonts()
    }
}

#[cfg(test)]
#[path = "system_tests.rs"]
mod tests;
