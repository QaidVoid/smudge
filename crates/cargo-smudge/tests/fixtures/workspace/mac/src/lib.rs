//! A procedural macro whose expansion reads the helper's thread-local,
//! so a change to the helper replays through the macro's own compile.

extern crate proc_macro;

use proc_macro::TokenStream;

/// Bakes the helper's tag into the output at expansion time.
#[proc_macro]
pub fn tag(_input: TokenStream) -> TokenStream {
    let baked = helper::tag();
    format!("\"{baked}\"")
        .parse()
        .expect("the baked tag is a string literal")
}
