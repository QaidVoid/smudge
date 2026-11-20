//! Tabs: a row of tabs over a stack of panels, one of which shows.

use smudge_core::element::{AnyElement, Element, IntoElement};
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Direction, Edges, pct};
use smudge_core::theme::{ColorToken, ColorValue};
use smudge_core::widget::Panel;
use smudge_text::label;

use crate::containers::{column, row};

/// Tabs, which [`tabs`] creates and [`Tabs::tab`] fills.
pub struct Tabs {
    /// The index of the tab whose panel shows.
    selected: RwSignal<usize>,
    /// The row of tabs, with the tabs added so far.
    list: Element<Panel>,
    /// The stack of panels, with the panels added so far.
    panels: Element<Panel>,
    /// How many tabs were added.
    count: usize,
}

impl Tabs {
    /// Returns the tabs with a tab named `name` added at the end, which
    /// shows the panel `content` builds.
    ///
    /// The panel is built once and stays mounted while another tab shows,
    /// hidden, so what it holds, such as the text of a field or where a
    /// list was scrolled to, is there again when its tab comes back.
    #[track_caller]
    pub fn tab<E: IntoElement>(
        mut self,
        name: impl Into<String>,
        content: impl FnOnce() -> E,
    ) -> Tabs {
        let name = name.into();
        let index = self.count;
        self.count += 1;
        let (state, color, mark, press, focus) = (
            self.selected.clone(),
            self.selected.clone(),
            self.selected.clone(),
            self.selected.clone(),
            self.selected.clone(),
        );
        let tab = row()
            .padding_xy(Size {
                width: 12.0,
                height: 6.0,
            })
            .border_edges(Edges {
                top: 0.0,
                right: 0.0,
                bottom: 2.0,
                left: 0.0,
            })
            .border_color(move || {
                if mark.get() == index {
                    ColorValue::Token(ColorToken::ACCENT)
                } else {
                    ColorValue::Literal(Color::TRANSPARENT)
                }
            })
            .hover(|p| p.background(ColorToken::SURFACE_HOVER))
            .focused(|p| p.background(ColorToken::SURFACE_HOVER))
            .role(Role::Tab)
            .accessible_label(name.clone())
            .accessible_selected(move || state.get() == index)
            .focusable()
            .activate_on_keys()
            .on_press(move || press.set(index))
            // A tab the arrows move to shows its panel at once.
            .on_focus(move |focused| {
                if focused && focus.get() != index {
                    focus.set(index);
                }
            })
            .child(label(name.clone()).text_color(move || {
                ColorValue::from(if color.get() == index {
                    ColorToken::TEXT
                } else {
                    ColorToken::TEXT_MUTED
                })
            }));
        let shown = self.selected.clone();
        let panel = column()
            .grow(1.0)
            .width(pct(100.0))
            .role(Role::TabPanel)
            .accessible_label(name)
            .hidden(move || shown.get() != index)
            .child(content());
        self.list = self.list.child(tab);
        self.panels = self.panels.child(panel);
        self
    }
}

impl IntoElement for Tabs {
    fn into_any(self) -> AnyElement {
        column()
            .child(self.list)
            .child(self.panels.grow(1.0))
            .into_any()
    }
}

/// Creates tabs whose shown panel is the one `selected` names by index,
/// which [`Tabs::tab`] fills.
///
/// A press on a tab shows its panel. Left and Right move along the tabs and
/// show each one's panel as they reach it, and Home and End go to the first
/// and the last. Tab stops once on the row, at the shown tab. Screen readers
/// hear a tab list of tabs, the shown one selected, each with its panel. An
/// index past the last tab shows no panel and selects no tab.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_text::label;
/// use smudge_widgets::tabs;
///
/// let shown = RwSignal::new(0_usize);
/// let side = tabs(shown)
///     .tab("Entities", || label("The entities"))
///     .tab("Textures", || label("The textures"));
/// ```
pub fn tabs(selected: RwSignal<usize>) -> Tabs {
    Tabs {
        selected,
        list: row()
            .role(Role::TabList)
            .navigate_with_arrows(Direction::Row)
            .border_edges(Edges {
                top: 0.0,
                right: 0.0,
                bottom: 1.0,
                left: 0.0,
            })
            .border_color(ColorToken::BORDER),
        panels: column(),
        count: 0,
    }
}
