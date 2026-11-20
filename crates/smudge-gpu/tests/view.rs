//! App-drawn GPU content, composited by the GPU renderer, and its CPU
//! fallback.
//!
//! The GPU tests need an adapter and skip cleanly, with a printed reason,
//! where there is none.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::geometry::{Color, Size};
use smudge_core::image::Image;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::renderer::Renderer;
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::panel;
use smudge_cpu::CpuRenderer;
use smudge_gpu::wgpu::{self, Device, Queue};
use smudge_gpu::{GpuFrame, GpuRenderer, GpuViewExt, gpu_view};

/// The viewport the scenes mount in, in logical pixels.
const VIEWPORT: Size = Size {
    width: 80.0,
    height: 60.0,
};

/// Requests a device on the default adapter, or `None` without one.
fn headless_device() -> Option<(Device, Queue)> {
    let options = wgpu::RequestAdapterOptions::default();
    let adapter =
        futures_executor::block_on(smudge_gpu::instance().request_adapter(&options)).ok()?;
    futures_executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: smudge_gpu::device_limits(&adapter),
        ..Default::default()
    }))
    .ok()
}

/// Clears `frame`'s texture to `color`.
fn clear(frame: &GpuFrame<'_>, color: Color) {
    let channel = |value: u8| f64::from(value) / 255.0;
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: channel(color.r),
                        g: channel(color.g),
                        b: channel(color.b),
                        a: channel(color.a),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    frame.queue.submit([encoder.finish()]);
}

/// An app showing a view at 20, 10, 40 by 30 logical pixels that clears to
/// red, or blue while `blue` reads `true`, and counts its draws in `draws`.
/// Under the CPU renderer it shows `fallback`.
fn app(blue: RwSignal<bool>, draws: Rc<Cell<u32>>, fallback: Option<Image>) -> App {
    App::new(
        AppConfig {
            viewport: VIEWPORT,
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || {
            let (wanted, drawn) = (blue.clone(), blue.clone());
            let draws = Rc::clone(&draws);
            panel().child(
                gpu_view(move |frame| {
                    draws.set(draws.get() + 1);
                    let color = if wanted.get() {
                        Color::rgb(0, 0, 255)
                    } else {
                        Color::rgb(255, 0, 0)
                    };
                    clear(&frame, color);
                })
                .redraw_when(move || drawn.get())
                .fallback(fallback.clone())
                .absolute()
                .inset(Edges {
                    left: Length::px(20.0),
                    top: Length::px(10.0),
                    right: Length::Auto,
                    bottom: Length::Auto,
                })
                .width(px(40.0))
                .height(px(30.0)),
            )
        },
    )
}

/// Returns the straight RGBA of the pixel at `x`, `y` of `pixels`, a
/// surface `width` pixels wide.
fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
fn the_renderer_composites_what_the_view_drew_where_the_view_is() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    let blue = RwSignal::new(false);
    let draws = Rc::new(Cell::new(0));
    let mut app = app(blue.clone(), Rc::clone(&draws), None);
    let (width, height) = app.surface_size();
    let mut renderer = GpuRenderer::new(device, queue, width, height);

    renderer.render(&app.frame());
    let pixels = renderer.read_pixels();
    assert_eq!(pixel(&pixels, width, 40, 25), [255, 0, 0, 255], "inside");
    assert_eq!(
        pixel(&pixels, width, 10, 25),
        [255, 255, 255, 255],
        "left of it"
    );
    assert_eq!(
        pixel(&pixels, width, 40, 45),
        [255, 255, 255, 255],
        "below it"
    );
    assert_eq!(draws.get(), 1);

    renderer.render(&app.frame());
    assert_eq!(draws.get(), 1, "an unchanged view keeps its drawing");

    blue.set(true);
    renderer.render(&app.frame());
    assert_eq!(draws.get(), 2, "a change of what it reads draws it again");
    let pixels = renderer.read_pixels();
    assert_eq!(pixel(&pixels, width, 40, 25), [0, 0, 255, 255]);
}

#[test]
fn the_cpu_renderer_shows_the_fallback_image() {
    let red = Image::from_straight_rgba(2, 2, [200, 40, 40, 255].repeat(4))
        .expect("the buffer holds four pixels");
    let draws = Rc::new(Cell::new(0));
    let mut app = app(RwSignal::new(false), Rc::clone(&draws), Some(red));
    let (width, height) = app.surface_size();
    let mut renderer = CpuRenderer::new(width, height);

    renderer.render(&app.frame());

    let pixels = renderer.pixels();
    assert_eq!(pixel(pixels, width, 40, 25), [200, 40, 40, 255]);
    assert_eq!(pixel(pixels, width, 10, 25), [255, 255, 255, 255]);
    assert_eq!(
        draws.get(),
        0,
        "the CPU renderer never calls the GPU closure"
    );
}

#[test]
fn a_moving_fallback_repaints_exactly_what_a_full_repaint_draws() {
    let left = RwSignal::new(20.0_f32);
    let moved = left.clone();
    let mut harness = smudge_test::Harness::new(120.0, 60.0, 1.5, move || {
        let moved = moved.clone();
        let red = Image::from_straight_rgba(2, 2, [200, 40, 40, 255].repeat(4))
            .expect("the buffer holds four pixels");
        panel().child(
            gpu_view(|_| {})
                .fallback(Some(red))
                .absolute()
                .inset(move || Edges {
                    left: Length::px(moved.get()),
                    top: Length::px(10.0),
                    right: Length::Auto,
                    bottom: Length::Auto,
                })
                .width(px(30.0))
                .height(px(20.0)),
        )
    });
    harness.frame();
    left.set(65.0);
    harness.frame();

    assert!(harness.frame().damage.is_empty());
    harness.assert_matches_full_repaint();
}
