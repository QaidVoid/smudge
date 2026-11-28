use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, KeepAlive, Layer, Placement, Slot, for_each};
use smudge_core::geometry::{Point, Rect};
use smudge_core::input::{Handled, Key, KeyEvent};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::{Memo, RenderEffect, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Align, Edges, Justify, Length, Overflow, px};
use smudge_core::theme::ColorToken;
use smudge_core::widget::{PaintCx, Panel, Widget, panel};
use smudge_core::window::{Cursor, ResizeEdge};
use smudge_text::label;

use super::layout::{MIN_FLOAT_HEIGHT, MIN_FLOAT_WIDTH};
use super::widget::{State, fill};
use super::{keyboard, strip};
use crate::signal;

const EDGE: f32 = 5.0;
const GRIP: f32 = 25.0;
const STEP: f32 = 8.0;

pub(super) fn mount(state: Rc<State>, store: KeepAlive) -> Element<Slot> {
    let list = Rc::clone(&state);
    let keys = Memo::new(move || {
        list.layout.with(|layout| {
            layout
                .floating()
                .iter()
                .map(|group| group.panel().to_owned())
                .collect::<Vec<_>>()
        })
    });
    for_each(
        move || keys.get(),
        Clone::clone,
        move |key| {
            let placement_state = Rc::clone(&state);
            let placement_key = key.clone();
            let state = Rc::clone(&state);
            let store = store.clone();
            panel().overlay_in(
                Layer::Floating,
                signal(true),
                move || {
                    Placement::Window(
                        placement_state
                            .floating_rect(&placement_key)
                            .unwrap_or_default()
                            .origin(),
                    )
                },
                move || shell(Rc::clone(&state), &store, key.clone()),
            )
        },
    )
    .absolute()
    .width(px(0.0))
    .height(px(0.0))
}

struct Mode {
    original: Rect,
    pending: Option<Rect>,
    resize: bool,
}

pub(super) struct Window {
    state: Rc<State>,
    key: String,
    frame: Rc<Cell<Rect>>,
    mode: Cell<Option<Mode>>,
}

impl Window {
    pub(super) fn begin(&self, resize: bool) {
        let Some(original) = self.state.floating_rect(&self.key) else {
            return;
        };
        self.mode.set(Some(Mode {
            original,
            pending: None,
            resize,
        }));
        self.state.focus_for(&self.key).tab.request_focus(None);
        let action = if resize { "resize" } else { "move" };
        self.state.announce(format!(
            "Use arrow keys to {action} {}. Enter to keep, Escape to cancel.",
            self.state.title(&self.key)
        ));
    }

    pub(super) fn end(&self) {
        self.mode.set(None);
    }

    pub(super) fn key(&self, event: &KeyEvent) -> Handled {
        if !event.pressed {
            return Handled::No;
        }
        let Some(mut mode) = self.mode.take() else {
            return Handled::No;
        };
        let (x, y) = match event.key {
            Key::Enter if !event.repeat => return Handled::Yes,
            Key::Escape => {
                self.state.set_floating_rect(&self.key, mode.original);
                return Handled::Yes;
            }
            Key::ArrowLeft => (-STEP, 0.0),
            Key::ArrowRight => (STEP, 0.0),
            Key::ArrowUp => (0.0, -STEP),
            Key::ArrowDown => (0.0, STEP),
            _ => {
                self.mode.set(Some(mode));
                return Handled::No;
            }
        };
        let edge = mode.resize.then_some(ResizeEdge::SouthEast);
        let rect = mode.pending.unwrap_or_else(|| self.frame.get());
        let requested = change_rect(rect, edge, Point { x, y });
        if self.state.set_floating_rect(&self.key, requested) {
            mode.pending = Some(requested);
        }
        self.mode.set(Some(mode));
        Handled::Yes
    }

    pub(super) fn grip(self: &Rc<Self>) -> Element<impl Widget> {
        self.handle(None)
            .width(px(GRIP))
            .shrink(0.0)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .child(label("⋮").text_color(ColorToken::TEXT_MUTED))
            .background(ColorToken::SURFACE_HOVER)
    }

    fn handle(self: &Rc<Self>, edge: Option<ResizeEdge>) -> Element<Frame> {
        let rect = Rc::new(Cell::new(Rect::default()));
        let at = Rc::clone(&rect);
        let position = move |local: Point| {
            let origin = at.get().origin();
            Point {
                x: local.x + origin.x,
                y: local.y + origin.y,
            }
        };
        let drag = Rc::new(Cell::new(None::<(Rect, Point)>));
        let start = Rc::clone(&drag);
        let end = Rc::clone(&drag);
        let window = Rc::clone(self);
        let moving = Rc::clone(self);
        let origin = position.clone();
        Element::new(Frame(rect))
            .keeps_focus()
            .on_pointer_down(move |local| {
                window.end();
                start.set(Some((window.frame.get(), origin(local))));
            })
            .on_pointer_move(move |local| {
                if let Some((rect, start)) = drag.get() {
                    let now = position(local);
                    let delta = Point {
                        x: now.x - start.x,
                        y: now.y - start.y,
                    };
                    moving
                        .state
                        .set_floating_rect(&moving.key, change_rect(rect, edge, delta));
                }
            })
            .on_pointer_up(move |_| end.set(None))
    }
}

struct Frame(Rc<Cell<Rect>>);

impl Widget for Frame {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        self.0.set(cx.rect);
        Panel.paint(cx, out);
    }
}

fn shell(state: Rc<State>, store: &KeepAlive, key: String) -> Element<Frame> {
    let window = Rc::new(Window {
        state: Rc::clone(&state),
        key: key.clone(),
        frame: Rc::new(Cell::new(Rect::default())),
        mode: Cell::new(None),
    });
    let applied = Rc::clone(&window);
    RenderEffect::new(move || {
        applied.state.floating_rect(&applied.key);
        if let Some(mut mode) = applied.mode.take() {
            mode.pending = None;
            applied.mode.set(Some(mode));
        }
    });
    let keys = {
        let key = key.clone();
        Memo::new(move || vec![key.clone()])
    };
    let width = Rc::clone(&window);
    let height = Rc::clone(&window);
    let title = Rc::clone(&window);
    let focus = Rc::clone(&window);
    let mut frame = Element::new(Frame(Rc::clone(&window.frame)))
        .column()
        .width(move || {
            px(width
                .state
                .floating_rect(&width.key)
                .unwrap_or_default()
                .width())
        })
        .height(move || {
            px(height
                .state
                .floating_rect(&height.key)
                .unwrap_or_default()
                .height())
        })
        .overflow(Overflow::Clip)
        .background(ColorToken::SURFACE)
        .padding(px(EDGE))
        .border_color(ColorToken::BORDER)
        .border(1.0)
        .role(Role::Group)
        .accessible_label(move || title.state.title(&title.key))
        .on_unhandled_key(move |event| {
            keyboard::group_key(&focus.state, None, Some(&focus.key), event)
        })
        .child(strip::strip(None, state, keys, Some(Rc::clone(&window))))
        .child(fill(store.slot(key)));
    for (edge, [top, right, bottom, left]) in [
        (ResizeEdge::North, [0.0, EDGE, -1.0, EDGE]),
        (ResizeEdge::South, [-1.0, EDGE, 0.0, EDGE]),
        (ResizeEdge::West, [EDGE, -1.0, EDGE, 0.0]),
        (ResizeEdge::East, [EDGE, 0.0, EDGE, -1.0]),
        (ResizeEdge::NorthWest, [0.0, -1.0, -1.0, 0.0]),
        (ResizeEdge::NorthEast, [0.0, 0.0, -1.0, -1.0]),
        (ResizeEdge::SouthWest, [-1.0, -1.0, 0.0, 0.0]),
        (ResizeEdge::SouthEast, [-1.0, 0.0, 0.0, -1.0]),
    ] {
        let inset = |n| if n < 0.0 { Length::Auto } else { px(n) };
        let mut handle = window
            .handle(Some(edge))
            .absolute()
            .cursor(Cursor::Resize(edge))
            .inset(Edges {
                top: inset(top),
                right: inset(right),
                bottom: inset(bottom),
                left: inset(left),
            });
        if left < 0.0 || right < 0.0 {
            handle = handle.width(px(EDGE));
        }
        if top < 0.0 || bottom < 0.0 {
            handle = handle.height(px(EDGE));
        }
        frame = frame.child(handle);
    }
    frame
}

fn change_rect(mut rect: Rect, edge: Option<ResizeEdge>, delta: Point) -> Rect {
    use ResizeEdge::*;
    let Some(edge) = edge else {
        return rect.translated(delta);
    };
    if matches!(edge, West | NorthWest | SouthWest) {
        rect.x0 = (rect.x0 + delta.x).min(rect.x1 - MIN_FLOAT_WIDTH);
    }
    if matches!(edge, East | NorthEast | SouthEast) {
        rect.x1 = (rect.x1 + delta.x).max(rect.x0 + MIN_FLOAT_WIDTH);
    }
    if matches!(edge, North | NorthEast | NorthWest) {
        rect.y0 = (rect.y0 + delta.y).min(rect.y1 - MIN_FLOAT_HEIGHT);
    }
    if matches!(edge, South | SouthEast | SouthWest) {
        rect.y1 = (rect.y1 + delta.y).max(rect.y0 + MIN_FLOAT_HEIGHT);
    }
    rect
}
