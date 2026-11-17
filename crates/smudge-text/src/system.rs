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
    /// Scratch space reused across rich layouts, whose brush is the index of
    /// the span a run belongs to.
    pub(crate) rich_layouts: parley::LayoutContext<u32>,
}

impl TextSystem {
    /// Creates a system that sees only the bundled fonts.
    ///
    /// Without the `bundled-fonts` feature the system starts empty and every
    /// glyph lays out as missing.
    pub fn bundled_only() -> TextSystem {
        let mut system = TextSystem::empty();
        system.register_bundled_fonts();
        system
    }

    /// Creates a system that sees no fonts at all.
    fn empty() -> TextSystem {
        let collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let source_cache =
            fontique::SourceCache::new(fontique::SourceCacheOptions { shared: false });

        TextSystem {
            fonts: parley::FontContext {
                collection,
                source_cache,
            },
            layouts: parley::LayoutContext::new(),
            rich_layouts: parley::LayoutContext::new(),
        }
    }

    /// Creates a system that sees the system fonts, with the bundled ones as a
    /// fallback when the `bundled-fonts` feature is on.
    ///
    /// System fonts are discovered only when the `system-fonts` feature is
    /// enabled, and only on Linux, FreeBSD, Windows, Android and Apple
    /// platforms. Without it this behaves like [`TextSystem::bundled_only`].
    /// Without `bundled-fonts`, text depends entirely on the faces the host
    /// has.
    pub fn with_system_fonts() -> TextSystem {
        let mut system = TextSystem {
            fonts: parley::FontContext::default(),
            layouts: parley::LayoutContext::new(),
            rich_layouts: parley::LayoutContext::new(),
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
    // Kept out of line, so an application crate never inlines the access to
    // the thread-local, for the reason `with_current` gives.
    #[inline(never)]
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
        let mut f = Some(f);
        let mut result = None;
        with_current(&mut |system| {
            let f = f.take().expect("with_current calls its closure once");
            result = Some(f(system));
        });
        result.expect("with_current calls its closure once")
    }

    /// Registers the font faces in `data`, a font file or a font collection
    /// file, and answers the names of the families they belong to, in the
    /// order the file holds them.
    ///
    /// Text finds a registered face by its family name, through
    /// [`TextStyle::family`](crate::TextStyle::family) and
    /// [`FontFamily::named`](crate::FontFamily::named), and
    /// [`TextSystem::prefer_family`] makes it what a generic family such as
    /// sans serif resolves to first. This is how an app ships fonts of its
    /// own, embedded with `include_bytes!`, on native targets and on the web,
    /// where nothing else brings fonts besides the bundled faces.
    ///
    /// A face joins the family of its name that was registered before, a
    /// bundled one included, behind that family's faces: for each character,
    /// text uses the first face of the right weight, style and width that
    /// has a glyph for it, so a later file covers what earlier ones lack, as
    /// a font split into one file per script expects. A registered family
    /// takes the place of an installed family of the same name, so text that
    /// names it lays out the same on every machine. Register every face of
    /// it the app uses.
    ///
    /// Each thread has a system of its own, so register fonts on the thread
    /// that runs the window, before it shows. Text laid out before a
    /// registration keeps the faces it found until its style or its text
    /// changes. The system keeps registered data for as long as the thread
    /// lives, so register a font once.
    ///
    /// ```
    /// use smudge_text::{FontError, GenericFamily, TextSystem};
    ///
    /// /// Registers the app's font, embedded with
    /// /// `include_bytes!("../assets/Brand.ttf")`, and makes it the first
    /// /// choice of sans serif text.
    /// fn use_brand_font(data: &'static [u8]) -> Result<(), FontError> {
    ///     TextSystem::with(|system| {
    ///         for family in system.register_font(data)? {
    ///             system.prefer_family(GenericFamily::SansSerif, &family)?;
    ///         }
    ///         Ok(())
    ///     })
    /// }
    ///
    /// assert_eq!(use_brand_font(&[]), Err(FontError::NoFace));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`FontError::NoFace`] when `data` holds no face the system can
    /// read.
    pub fn register_font(
        &mut self,
        data: impl AsRef<[u8]> + Send + Sync + 'static,
    ) -> Result<Vec<String>, FontError> {
        let blob = fontique::Blob::new(std::sync::Arc::new(data));
        let collection = &mut self.fonts.collection;
        let mut registered = collection.register_fonts(blob, None);
        if registered.is_empty() {
            return Err(FontError::NoFace);
        }
        registered.sort_by_key(|(_, faces)| faces.iter().map(|face| face.index()).min());
        Ok(registered
            .into_iter()
            .filter_map(|(family, _)| collection.family_name(family).map(str::to_owned))
            .collect())
    }

    /// Makes the family named `family` the first choice of `generic`, so
    /// text whose style names the generic family uses it, and the families
    /// behind it cover the characters it lacks.
    ///
    /// The family is a registered one, an installed one when system fonts
    /// are on, or a bundled one. Naming a family that is already among the
    /// choices moves it to the front.
    ///
    /// # Errors
    ///
    /// Returns [`FontError::UnknownFamily`] when no font the system knows
    /// belongs to `family`.
    pub fn prefer_family(
        &mut self,
        generic: crate::GenericFamily,
        family: &str,
    ) -> Result<(), FontError> {
        let collection = &mut self.fonts.collection;
        let id = collection
            .family_id(family)
            .ok_or(FontError::UnknownFamily)?;
        let mut families = vec![id];
        for other in collection.generic_families(generic) {
            if !families.contains(&other) {
                families.push(other);
            }
        }
        collection.set_generic_families(generic, families.into_iter());
        Ok(())
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
        let mono = register(self, crate::fonts::MONO_REGULAR);

        let sans_family = sans
            .first()
            .map(|(family, _)| *family)
            .expect("a font file yields at least one family");
        for face in [
            crate::fonts::SANS_BOLD,
            crate::fonts::SANS_ITALIC,
            crate::fonts::SANS_BOLD_ITALIC,
        ] {
            let family = register(self, face)
                .first()
                .map(|(family, _)| *family)
                .expect("a font file yields at least one family");
            debug_assert_eq!(
                sans_family, family,
                "the bundled sans faces share a family name"
            );
        }
        let mono_family = mono
            .first()
            .map(|(family, _)| *family)
            .expect("a font file yields at least one family");

        self.fall_back_to(fontique::GenericFamily::SansSerif, sans_family);
        self.fall_back_to(fontique::GenericFamily::SystemUi, sans_family);
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
        // The collection answers the families set here followed by the
        // system's own, so the system's appear twice once a list is set.
        let mut families = Vec::new();
        for other in collection.generic_families(generic) {
            if other != family && !families.contains(&other) {
                families.push(other);
            }
        }
        families.push(family);
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

/// Runs `f` once with the current thread's system, creating one on first
/// use.
///
/// [`TextSystem::with`] is generic, so it is compiled in every crate that
/// calls it. It reaches the thread-local through this function, which is
/// neither generic nor inlined, so code compiled in an application crate
/// never addresses the thread-local itself. A hot patch is built from the
/// application's crates, and a patch that addressed it would carry a fresh,
/// empty copy of the text system instead of the running one.
///
/// # Panics
///
/// Panics if the thread's system is already handed out.
#[inline(never)]
fn with_current(f: &mut dyn FnMut(&mut TextSystem)) {
    CURRENT.with(|slot| {
        let mut slot = slot.try_borrow_mut().expect(
            "TextSystem::with cannot run inside another with call on the \
             same thread",
        );
        f(slot.get_or_insert_with(default_for_platform));
    });
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

/// Why a font could not be registered or chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontError {
    /// The data holds no face the text system can read.
    NoFace,
    /// No font the text system knows belongs to the family.
    UnknownFamily,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::NoFace => f.write_str("the data holds no font face"),
            FontError::UnknownFamily => f.write_str("no font belongs to the family"),
        }
    }
}

impl std::error::Error for FontError {}

#[cfg(test)]
#[path = "system_tests.rs"]
mod tests;
