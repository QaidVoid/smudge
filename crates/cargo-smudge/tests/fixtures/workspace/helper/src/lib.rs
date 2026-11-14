//! A library only a procedural macro reads at expansion time, holding a
//! thread-local the running app never links.

thread_local! {
    static TAG: &'static str = "macro-h-1";
}

/// The tag the macro bakes into its output at expansion time.
#[inline(never)]
pub fn tag() -> &'static str {
    TAG.with(|tag| *tag)
}
