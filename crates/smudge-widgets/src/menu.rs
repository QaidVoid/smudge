//! Menus: a menu bar, menu buttons, context menus, and the items, check
//! items, separators and submenus they hold.
//!
//! A menu opens in an overlay, so it escapes every clip around the node
//! that opens it. The keyboard walks it: Up and Down step through the
//! items, a letter goes to the next item that starts with it, Enter and
//! Space choose, Right opens a submenu and Left closes it, Escape closes
//! the menu that is open, and in a menu bar Left and Right move to the
//! next menu. A press outside closes every menu.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::element::{Element, IntoElement, Placement, Value};
use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Handled, Key};
use smudge_core::paint::PaintOp;
use smudge_core::path::{LineCap, LineJoin, Path, PathPaint, Stroke};
use smudge_core::reactive::{Owner, RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role};
use smudge_core::style::{Align, Direction, px};
use smudge_core::theme::{ColorToken, ColorValue, RadiusToken};
use smudge_core::tree::Dirty;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_text::label;

use crate::containers::{column, row};

/// What choosing a menu item does.
#[derive(Clone)]
enum Kind {
    /// Runs the closure.
    Action(Rc<dyn Fn()>),
    /// Toggles the signal.
    Check(RwSignal<bool>),
    /// Opens a menu of the items the closure returns.
    Submenu(Rc<dyn Fn() -> Vec<MenuItem>>),
    /// Nothing: a line between groups of items.
    Separator,
}

/// One entry of a menu, built by [`menu_item`], [`menu_check_item`],
/// [`submenu`] or [`menu_separator`].
#[derive(Clone)]
pub struct MenuItem {
    /// What choosing the item does.
    kind: Kind,
    /// The item's text.
    label: String,
    /// The shortcut the item shows, if any.
    shortcut: Option<String>,
    /// Whether the item is disabled.
    disabled: Value<bool>,
}

impl MenuItem {
    /// Returns the item showing `shortcut`, such as `Ctrl+S`, at its right.
    ///
    /// The text only tells the user: the app handles the shortcut itself,
    /// as a key handler around the content does.
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> MenuItem {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Returns the item disabled while `disabled` reads `true`, so it can be
    /// neither chosen nor focused.
    pub fn disabled(mut self, disabled: impl Into<Value<bool>>) -> MenuItem {
        self.disabled = disabled.into();
        self
    }
}

/// Creates an item that runs `on_choose` when it is chosen, after the menus
/// close, under the owner the menu's opener was built under, so the signals
/// and timers it creates live as long as the opener does.
pub fn menu_item(label: impl Into<String>, on_choose: impl Fn() + 'static) -> MenuItem {
    MenuItem {
        kind: Kind::Action(Rc::new(on_choose)),
        label: label.into(),
        shortcut: None,
        disabled: Value::Static(false),
    }
}

/// Creates an item that shows a check mark while `checked` reads `true`,
/// and toggles it when it is chosen.
pub fn menu_check_item(label: impl Into<String>, checked: RwSignal<bool>) -> MenuItem {
    MenuItem {
        kind: Kind::Check(checked),
        label: label.into(),
        shortcut: None,
        disabled: Value::Static(false),
    }
}

/// Creates an item that opens a menu of the items `items` returns beside
/// it. `items` runs each time the submenu opens, so it can follow the app's
/// state.
pub fn submenu(label: impl Into<String>, items: impl Fn() -> Vec<MenuItem> + 'static) -> MenuItem {
    MenuItem {
        kind: Kind::Submenu(Rc::new(items)),
        label: label.into(),
        shortcut: None,
        disabled: Value::Static(false),
    }
}

/// Creates a line between groups of items.
pub fn menu_separator() -> MenuItem {
    MenuItem {
        kind: Kind::Separator,
        label: String::new(),
        shortcut: None,
        disabled: Value::Static(false),
    }
}

/// How the top menu of a menu bar moves to the menu beside it.
#[derive(Clone)]
struct Bar {
    /// The open signal of every menu of the bar, in order.
    menus: Rc<RefCell<Vec<RwSignal<bool>>>>,
    /// The index of the menu this one is.
    index: usize,
}

impl Bar {
    /// Closes this menu and opens the one `step` menus along, wrapping.
    fn step(&self, step: isize) {
        let menus = self.menus.borrow();
        let count = menus.len() as isize;
        if count < 2 {
            return;
        }
        let next = (self.index as isize + step).rem_euclid(count) as usize;
        menus[self.index].set(false);
        menus[next].set(true);
    }
}

/// Where a menu sits among the menus open together.
#[derive(Clone)]
struct Level {
    /// Closes every menu open together, as choosing an item does.
    close_all: Rc<dyn Fn()>,
    /// The signal of this menu, which Left closes, for a submenu.
    own: Option<RwSignal<bool>>,
    /// The menu bar this menu opens from, for a top menu of a bar.
    bar: Option<Bar>,
    /// The owner the menu's opener was built under, which the chosen
    /// item's action runs in, so what the action creates outlives the menu.
    owner: Option<Owner>,
}

impl Level {
    /// Closes every menu open together, then runs `action` under the owner
    /// the menus' opener was built under.
    fn choose(&self, action: impl FnOnce()) {
        (self.close_all)();
        match &self.owner {
            Some(owner) => owner.with(action),
            None => action(),
        }
    }
}

/// Returns whether `item` can take the focus as the menu opens: it is no
/// separator and not disabled now.
fn focusable_now(item: &MenuItem) -> bool {
    let disabled = match &item.disabled {
        Value::Static(disabled) => *disabled,
        Value::Dynamic(get) => get(),
    };
    !matches!(item.kind, Kind::Separator) && !disabled
}

/// The width of the column the check mark sits in, in logical pixels.
const CHECK: f32 = 14.0;

/// The check mark of a check item, in the item's text color.
struct CheckMark {
    /// Whether the mark shows.
    checked: bool,
}

/// The key of a check mark's binding.
struct Checks;

impl Widget for CheckMark {
    /// Strokes a tick across the content box while checked, in the node's
    /// text color, or the theme's text color without one.
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        if !self.checked {
            return;
        }
        let color = cx
            .style
            .paint
            .text_color
            .unwrap_or(ColorValue::Token(ColorToken::TEXT))
            .resolve(cx.theme);
        let area = cx.content;
        let at = |x: f32, y: f32| Point {
            x: area.x0 + x * area.width(),
            y: area.y0 + y * area.height(),
        };
        let tick = Path::polyline([at(0.15, 0.55), at(0.4, 0.78), at(0.85, 0.25)]);
        out.push(PaintOp::Path(Rc::new(PathPaint::stroked(
            tick,
            Stroke::new(1.8, color)
                .with_cap(LineCap::Round)
                .with_join(LineJoin::Round),
        ))));
    }
}

/// Builds the check mark of a check item that shows while `checked` reads
/// `true`.
fn check_mark(checked: RwSignal<bool>) -> Element<CheckMark> {
    Element::new(CheckMark { checked: false })
        .bind(
            Checks,
            move || checked.get(),
            |mark, checked| {
                mark.checked = checked;
                Dirty::Paint
            },
        )
        .width(px(CHECK))
        .height(px(CHECK))
        .shrink(0.0)
}

/// Builds the menu of `items`, at `level` among the menus open together.
fn menu_panel(items: Vec<MenuItem>, level: Level) -> Element<Panel> {
    let siblings: Rc<RefCell<Vec<RwSignal<bool>>>> = Rc::new(RefCell::new(Vec::new()));
    let (own, bar) = (level.own.clone(), level.bar.clone());
    let mut menu = column()
        .role(Role::Menu)
        .padding(px(4.0))
        .min_width(px(160.0))
        .background(ColorToken::SURFACE)
        .border(1.0)
        .border_color(ColorToken::BORDER)
        .corner_radius(RadiusToken::MEDIUM)
        .trap_focus()
        .navigate_with_arrows(Direction::Column)
        .on_key(move |event| {
            if !event.pressed {
                return Handled::No;
            }
            match (&event.key, &own, &bar) {
                (Key::ArrowLeft, Some(own), _) => {
                    own.set(false);
                    Handled::Yes
                }
                (Key::ArrowLeft, None, Some(bar)) => {
                    bar.step(-1);
                    Handled::Yes
                }
                (Key::ArrowRight, None, Some(bar)) => {
                    bar.step(1);
                    Handled::Yes
                }
                _ => Handled::No,
            }
        });
    // The first item that can take the focus takes it as the menu opens.
    let first = items.iter().position(focusable_now);
    for (index, item) in items.into_iter().enumerate() {
        menu = menu.child(entry(item, &level, &siblings, first == Some(index)));
    }
    menu
}

/// Builds one entry of a menu: an item, or a separator. The first entry
/// takes the focus when the menu opens.
fn entry(
    item: MenuItem,
    level: &Level,
    siblings: &Rc<RefCell<Vec<RwSignal<bool>>>>,
    first: bool,
) -> Element<Panel> {
    let MenuItem {
        kind,
        label: text,
        shortcut,
        disabled,
    } = item;
    if matches!(kind, Kind::Separator) {
        return panel()
            .height(px(1.0))
            .margin(px(4.0))
            .background(ColorToken::BORDER);
    }
    let sub_open = matches!(kind, Kind::Submenu(_)).then(|| {
        let open = RwSignal::new(false);
        let mut siblings = siblings.borrow_mut();
        siblings.push(open.clone());
        (siblings.len() - 1, open)
    });
    let role = if matches!(kind, Kind::Check(_)) {
        Role::MenuItemCheckBox
    } else {
        Role::MenuItem
    };
    let mut item = row()
        .gap(Size {
            width: 8.0,
            height: 0.0,
        })
        .align_items(Align::Center)
        .padding(px(5.0))
        .corner_radius(RadiusToken::SMALL)
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .focused(|p| p.background(ColorToken::SURFACE_HOVER))
        .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
        .disabled_style(|p| p.opacity(0.5))
        .role(role)
        .accessible_label(text.clone())
        .disabled(disabled)
        .focusable()
        .activate_on_keys();
    if first {
        item = item.autofocus();
    }
    item = match &kind {
        Kind::Check(checked) => {
            let state = checked.clone();
            item.accessible_checked(move || {
                if state.get() {
                    Checked::True
                } else {
                    Checked::False
                }
            })
            .child(check_mark(checked.clone()))
        }
        _ => item.child(panel().width(px(CHECK)).shrink(0.0)),
    };
    item = item.child(label(text).grow(1.0));
    if let Some(shortcut) = shortcut {
        item = item.child(label(shortcut).text_color(ColorToken::TEXT_MUTED));
    }
    // Hovering an item opens its submenu and closes the others of its menu.
    let (hovered_siblings, mine) = (Rc::clone(siblings), sub_open.as_ref().map(|(at, _)| *at));
    item = item.on_hover(move |inside| {
        if !inside {
            return;
        }
        for (at, open) in hovered_siblings.borrow().iter().enumerate() {
            let wanted = mine == Some(at);
            if open.get() != wanted {
                open.set(wanted);
            }
        }
    });
    match kind {
        Kind::Action(action) => {
            let level = level.clone();
            item.on_press(move || level.choose(|| action()))
        }
        Kind::Check(checked) => {
            let level = level.clone();
            item.on_press(move || level.choose(|| checked.update(|checked| *checked = !*checked)))
        }
        Kind::Submenu(items) => {
            let (_, open) = sub_open.expect("a submenu has its signal");
            let (press, right) = (open.clone(), open.clone());
            let inner = Level {
                close_all: Rc::clone(&level.close_all),
                own: Some(open.clone()),
                bar: None,
                owner: level.owner.clone(),
            };
            item.child(label("\u{203a}").text_color(ColorToken::TEXT_MUTED))
                .on_press(move || press.set(true))
                .on_key(move |event| {
                    // Keys from inside the open submenu bubble here too, and
                    // its Right belongs to the menu bar around it.
                    if event.pressed && event.key == Key::ArrowRight && !right.get() {
                        right.set(true);
                        return Handled::Yes;
                    }
                    Handled::No
                })
                .overlay(open, Placement::After, move || {
                    menu_panel(items(), inner.clone())
                })
        }
        Kind::Separator => item,
    }
}

/// Creates a button that opens a menu of the items `items` returns below
/// it.
///
/// `items` runs each time the menu opens, so it can follow the app's state.
/// The button toggles the menu, and so do Enter, Space and Down while it
/// holds the focus.
///
/// ```
/// use smudge_widgets::{menu, menu_item, menu_separator};
///
/// let file = menu("File", || {
///     vec![
///         menu_item("Open", || {}).shortcut("Ctrl+O"),
///         menu_separator(),
///         menu_item("Quit", || {}),
///     ]
/// });
/// ```
#[track_caller]
pub fn menu(
    title: impl Into<String>,
    items: impl Fn() -> Vec<MenuItem> + 'static,
) -> Element<Panel> {
    let open = crate::signal(false);
    title_item(title.into(), open, Rc::new(items), None)
}

/// Builds the title of a menu: an item that opens the menu of `items` below
/// it while `open` reads `true`.
fn title_item(
    title: String,
    open: RwSignal<bool>,
    items: Rc<dyn Fn() -> Vec<MenuItem>>,
    bar: Option<Bar>,
) -> Element<Panel> {
    let (press, key, shown) = (open.clone(), open.clone(), open.clone());
    let close = open.clone();
    let hover_bar = bar.clone();
    let level = Level {
        close_all: Rc::new(move || close.set(false)),
        own: None,
        bar,
        owner: Owner::current(),
    };
    row()
        .padding_xy(Size {
            width: 10.0,
            height: 5.0,
        })
        .corner_radius(RadiusToken::SMALL)
        .background(move || {
            smudge_core::theme::ColorValue::from(if shown.get() {
                ColorToken::SURFACE_PRESSED
            } else {
                ColorToken::SURFACE
            })
        })
        .hover(|p| p.background(ColorToken::SURFACE_HOVER))
        .focused(|p| p.background(ColorToken::SURFACE_HOVER))
        .role(Role::MenuItem)
        .accessible_label(title.clone())
        .focusable()
        .on_press(move || press.update(|open| *open = !*open))
        .on_key(move |event| {
            // Keys from inside the open menu bubble here too, and belong to
            // the menu.
            let opens = matches!(event.key, Key::Enter | Key::Space | Key::ArrowDown);
            if event.pressed && !event.repeat && opens && !key.get() {
                key.set(true);
                return Handled::Yes;
            }
            Handled::No
        })
        .on_hover(move |inside| {
            // Moving across a bar with a menu open opens the menu under the
            // pointer instead.
            let Some(bar) = &hover_bar else {
                return;
            };
            let menus = bar.menus.borrow();
            if inside && menus.iter().any(Get::get) && !menus[bar.index].get() {
                for open in menus.iter() {
                    open.set(false);
                }
                menus[bar.index].set(true);
            }
        })
        .child(label(title))
        .overlay(open, Placement::Below, move || {
            menu_panel(items(), level.clone())
        })
}

/// A menu bar, which [`menu_bar`] creates and [`MenuBar::menu`] fills.
pub struct MenuBar {
    /// The bar, with the titles added so far.
    bar: Element<Panel>,
    /// The open signal of every menu added so far, in order.
    menus: Rc<RefCell<Vec<RwSignal<bool>>>>,
}

impl MenuBar {
    /// Returns the bar with a menu titled `title` added at its end, holding
    /// the items `items` returns. `items` runs each time the menu opens, so
    /// it can follow the app's state.
    #[track_caller]
    pub fn menu(
        mut self,
        title: impl Into<String>,
        items: impl Fn() -> Vec<MenuItem> + 'static,
    ) -> MenuBar {
        let open = crate::signal(false);
        let index = {
            let mut menus = self.menus.borrow_mut();
            menus.push(open.clone());
            menus.len() - 1
        };
        let bar = Bar {
            menus: Rc::clone(&self.menus),
            index,
        };
        self.bar = self
            .bar
            .child(title_item(title.into(), open, Rc::new(items), Some(bar)));
        self
    }
}

impl IntoElement for MenuBar {
    fn into_any(self) -> smudge_core::element::AnyElement {
        self.bar.into_any()
    }
}

/// Creates an empty menu bar, which [`MenuBar::menu`] fills.
///
/// Left and Right move between the titles, and between the menus while one
/// is open, and moving the pointer across the bar while a menu is open
/// opens the menu under it.
///
/// ```
/// use smudge_core::reactive::RwSignal;
/// use smudge_widgets::{menu_bar, menu_check_item, menu_item};
///
/// let grid = RwSignal::new(true);
/// let bar = menu_bar()
///     .menu("File", || vec![menu_item("New", || {}).shortcut("Ctrl+N")])
///     .menu("View", move || vec![menu_check_item("Grid", grid.clone())]);
/// ```
pub fn menu_bar() -> MenuBar {
    MenuBar {
        bar: row()
            .gap(Size {
                width: 2.0,
                height: 0.0,
            })
            .padding(px(2.0))
            .role(Role::MenuBar)
            .navigate_with_arrows(Direction::Row),
        menus: Rc::new(RefCell::new(Vec::new())),
    }
}

/// The builder of a context menu.
pub trait ContextMenuExt: Sized {
    /// Opens a menu of the items `items` returns at the pointer on a right
    /// press, and at the top left of the focused node on the menu key or
    /// Shift+F10 while the node or a descendant holds the focus.
    ///
    /// `items` runs each time the menu opens, so it can follow the app's
    /// state.
    fn context_menu(self, items: impl Fn() -> Vec<MenuItem> + 'static) -> Self;
}

impl<W: smudge_core::widget::Widget + 'static> ContextMenuExt for Element<W> {
    #[track_caller]
    fn context_menu(self, items: impl Fn() -> Vec<MenuItem> + 'static) -> Self {
        let open = crate::signal(false);
        let at = crate::signal(Point::default());
        let (show, place, close) = (open.clone(), at.clone(), open.clone());
        let level = Level {
            close_all: Rc::new(move || close.set(false)),
            own: None,
            bar: None,
            owner: Owner::current(),
        };
        self.on_context_menu(move |point| {
            at.set(point);
            show.set(true);
        })
        .overlay(
            open,
            move || Placement::At(place.get()),
            move || menu_panel(items(), level.clone()),
        )
    }
}
