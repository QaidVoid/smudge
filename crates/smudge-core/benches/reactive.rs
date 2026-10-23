//! Compares frame-driven reactive updates against a hand-written dirty list.
//!
//! Both variants write a value, record which entry changed, and later push the
//! changed entries into one shared vector. The reactive variant pays for
//! waking and polling tasks. The baseline pays only for the writes and the
//! drain loop, so the difference is the cost of the runtime.

use criterion::{Criterion, criterion_group, criterion_main};
use reactive_graph::{
    effect::RenderEffect,
    owner::Owner,
    signal::signal_local,
    traits::{Get, Update},
};
use smudge_core::reactive;
use std::cell::{Cell, RefCell};
use std::hint::black_box;
use std::rc::Rc;

/// Number of independent values and effects in each variant.
const COUNT: usize = 10_000;

/// Benchmarks writing every signal and then flushing once, and writing one
/// signal and then flushing once.
fn reactive_updates(c: &mut Criterion) {
    reactive::init();
    let owner = Owner::new();
    owner.set();

    let mut writes = Vec::with_capacity(COUNT);
    let log = Rc::new(RefCell::new(Vec::with_capacity(COUNT)));
    let _effects: Vec<_> = (0..COUNT)
        .map(|index| {
            let (read, write) = signal_local(0u32);
            writes.push(write);
            let log = Rc::clone(&log);
            RenderEffect::new(move |_| log.borrow_mut().push((index, read.get())))
        })
        .collect();
    reactive::poll_local();

    let mut group = c.benchmark_group("reactive");
    group.bench_function("write 10000 then flush", |b| {
        b.iter(|| {
            log.borrow_mut().clear();
            for write in &writes {
                write.update(|value| *value += 1);
            }
            reactive::poll_local();
            black_box(&log);
        });
    });
    group.bench_function("write 1 then flush", |b| {
        b.iter(|| {
            log.borrow_mut().clear();
            writes[0].update(|value| *value += 1);
            reactive::poll_local();
            black_box(&log);
        });
    });
    group.finish();
}

/// Benchmarks the same work with plain cells and a dirty list.
fn dirty_list(c: &mut Criterion) {
    let cells: Vec<Cell<u32>> = (0..COUNT).map(|_| Cell::new(0)).collect();
    let log = Rc::new(RefCell::new(Vec::with_capacity(COUNT)));
    let mut dirty = Vec::with_capacity(COUNT);

    let mut group = c.benchmark_group("baseline");
    group.bench_function("write 10000 then drain", |b| {
        b.iter(|| {
            log.borrow_mut().clear();
            dirty.clear();
            for (index, cell) in cells.iter().enumerate() {
                cell.set(cell.get() + 1);
                dirty.push(index);
            }
            for index in dirty.drain(..) {
                log.borrow_mut().push((index, cells[index].get()));
            }
            black_box(&log);
        });
    });
    group.bench_function("write 1 then drain", |b| {
        b.iter(|| {
            log.borrow_mut().clear();
            cells[0].set(cells[0].get() + 1);
            log.borrow_mut().push((0, cells[0].get()));
            black_box(&log);
        });
    });
    group.finish();
}

criterion_group!(benches, reactive_updates, dirty_list);
criterion_main!(benches);
