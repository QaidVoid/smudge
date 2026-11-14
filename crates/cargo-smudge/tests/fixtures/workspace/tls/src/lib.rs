//! A library that keeps a thread-local, which a patch would give a fresh
//! empty copy of.

thread_local! {
    static LABEL: &'static str = "tls-1";
}

#[inline(never)]
pub fn label() -> &'static str {
    LABEL.with(|label| *label)
}
