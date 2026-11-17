#[cfg(all(feature = "hot-reload", target_os = "linux"))]
mod carried {
    use std::cell::RefCell;
    use std::rc::Rc;

    use smudge_core::geometry::Point;
    use smudge_core::hot::probe_of;
    use smudge_core::input::InputEvent;
    use smudge_core::style::px;
    use smudge_core::widget::panel;
    use smudge_test::Harness;

    use crate::virtual_list_keyed::{KeyedList, virtual_list_keyed};

    /// The true height of the row of `key`, a whole number between 20 and
    /// 200 that no estimate of 100 hides.
    fn row_height(key: u64) -> f32 {
        20.0 + (key.wrapping_mul(37) % 181) as f32
    }

    /// A list of 200 rows of their true heights in a 480 pixel window.
    fn build() -> impl Fn() -> smudge_core::element::AnyElement + Clone + 'static {
        build_estimating(100.0)
    }

    /// A list of 200 rows of their true heights in a 480 pixel window, which
    /// gives the rows it has not measured `estimate` pixels.
    fn build_estimating(
        estimate: f32,
    ) -> impl Fn() -> smudge_core::element::AnyElement + Clone + 'static {
        move || {
            use smudge_core::element::IntoElement;
            panel()
                .column()
                .child(
                    virtual_list_keyed(
                        || Rc::from((0..200).collect::<Vec<u64>>().as_slice()),
                        |item| *item,
                        estimate,
                        |item| panel().height(px(row_height(*item))),
                    )
                    .height(px(480.0)),
                )
                .into_any()
        }
    }

    /// Runs a frame, then the frames the list's paint-time reports ask for.
    fn settle(harness: &mut Harness) {
        harness.frame();
        for _ in 0..16 {
            if !harness.woken() {
                return;
            }
            harness.frame();
        }
        panic!("the list never settled");
    }

    /// The probes of every type a keyed list of `u64` items keeps in
    /// signals, apart from its own state when `with_state` is off.
    fn probes(with_state: bool) -> Vec<usize> {
        let mut probes = vec![
            probe_of::<f32>(),
            probe_of::<Option<f32>>(),
            probe_of::<Vec<u64>>(),
            probe_of::<Option<(usize, f32)>>(),
            probe_of::<u64>(),
            probe_of::<Option<crate::scroll::Viewport>>(),
        ];
        if with_state {
            probes.push(probe_of::<Rc<RefCell<KeyedList<u64, u64>>>>());
        }
        probes
    }

    /// Scrolls a list into its middle, remounts it after a patch that kept
    /// `probes`, and answers the layout before and after.
    fn remount_scrolled(probes: Vec<usize>) -> (String, String) {
        let build = build();
        let mut harness = Harness::new(640.0, 480.0, 1.0, build.clone());
        settle(&mut harness);
        harness.move_to(Point { x: 320.0, y: 240.0 });
        for _ in 0..12 {
            harness.event(InputEvent::Wheel {
                delta: Point { x: 0.0, y: 400.0 },
            });
            settle(&mut harness);
        }
        let before = harness.layout_dump();
        // SAFETY: the test never changes a type, so every layout it lists is
        // the one the values hold.
        unsafe { smudge_core::hot::note_kept_types(probes.into_iter().collect()) };
        harness.app().remount(build);
        settle(&mut harness);
        (before, harness.layout_dump())
    }

    /// The height of the list's content, from a layout dump.
    fn content_height(dump: &str) -> f32 {
        let line = dump
            .lines()
            .find(|line| line.trim_start().starts_with("ScrollContent"))
            .expect("the dump shows the scroll content");
        let numbers: Vec<f32> = line
            .split(['[', ']', ','])
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        numbers[3] - numbers[1]
    }

    /// The rows of a layout dump that sit inside the 480 pixel viewport.
    fn visible_rows(dump: &str) -> Vec<String> {
        dump.lines()
            .filter(|line| line.trim_start().starts_with("MeasureRow"))
            .filter(|line| {
                let numbers: Vec<f32> = line
                    .split(['[', ']', ','])
                    .filter_map(|part| part.trim().parse().ok())
                    .collect();
                numbers[3] > 0.0 && numbers[1] < 480.0
            })
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_remount_with_a_new_estimate_gives_it_to_the_rows_never_measured() {
        let mut harness = Harness::new(640.0, 480.0, 1.0, build_estimating(100.0));
        settle(&mut harness);
        harness.move_to(Point { x: 320.0, y: 240.0 });
        for _ in 0..12 {
            harness.event(InputEvent::Wheel {
                delta: Point { x: 0.0, y: 400.0 },
            });
            settle(&mut harness);
        }
        let before = harness.layout_dump();
        // SAFETY: the test never changes a type, so every layout it lists is
        // the one the values hold.
        unsafe { smudge_core::hot::note_kept_types(probes(true).into_iter().collect()) };
        harness.app().remount(build_estimating(20.0));
        settle(&mut harness);
        let after = harness.layout_dump();
        assert_eq!(
            visible_rows(&before),
            visible_rows(&after),
            "the view left the rows it showed"
        );
        // The rows never measured each shrink by 80 pixels.
        assert!(
            content_height(&after) < content_height(&before) - 10_000.0,
            "the carried heights kept the old estimate"
        );
    }

    #[test]
    fn a_remount_reconciles_carried_state_with_the_same_items() {
        use smudge_core::reactive::{RwSignal, prelude::*};

        // The items stay the very same allocation across the remount.
        let items: RwSignal<Rc<[u64]>> = RwSignal::new(Rc::from((0..200).collect::<Vec<u64>>()));
        let build = |estimate: f32, items: RwSignal<Rc<[u64]>>| {
            move || {
                use smudge_core::element::IntoElement;
                let items = items.clone();
                panel()
                    .column()
                    .child(
                        virtual_list_keyed(
                            move || items.get(),
                            |item| *item,
                            estimate,
                            |item| panel().height(px(row_height(*item))),
                        )
                        .height(px(480.0)),
                    )
                    .into_any()
            }
        };
        // An estimate of zero shows no rows and leaves the index empty.
        let mut harness = Harness::new(640.0, 480.0, 1.0, build(0.0, items.clone()));
        settle(&mut harness);
        // SAFETY: the test never changes a type, so every layout it lists is
        // the one the values hold.
        unsafe { smudge_core::hot::note_kept_types(probes(true).into_iter().collect()) };
        harness.app().remount(build(24.0, items));
        settle(&mut harness);
        assert!(
            !visible_rows(&harness.layout_dump()).is_empty(),
            "the carried empty index kept the list empty"
        );
    }

    #[test]
    fn a_remount_after_a_patch_keeps_what_a_shown_row_holds() {
        use smudge_core::reactive::{RwSignal, prelude::*};
        use std::collections::HashMap;

        // Each row keeps a value of its own, as a row's text field would.
        let values: Rc<RefCell<HashMap<u64, RwSignal<u32>>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let build = {
            let values = Rc::clone(&values);
            move || {
                use smudge_core::element::IntoElement;
                let values = Rc::clone(&values);
                panel()
                    .column()
                    .child(
                        virtual_list_keyed(
                            || Rc::from((0..200).collect::<Vec<u64>>().as_slice()),
                            |item| *item,
                            100.0,
                            move |item| {
                                values.borrow_mut().insert(*item, RwSignal::new(0u32));
                                panel().height(px(row_height(*item)))
                            },
                        )
                        .height(px(480.0)),
                    )
                    .into_any()
            }
        };
        let mut harness = Harness::new(640.0, 480.0, 1.0, build.clone());
        settle(&mut harness);
        values.borrow()[&2].set(42);
        let mut kept = probes(true);
        kept.push(probe_of::<u32>());
        // SAFETY: the test never changes a type, so every layout it lists is
        // the one the values hold.
        unsafe { smudge_core::hot::note_kept_types(kept.into_iter().collect()) };
        harness.app().remount(build);
        settle(&mut harness);
        assert_eq!(
            values.borrow()[&2].read(|value| *value),
            42,
            "a shown row lost what it held"
        );
    }

    thread_local! {
        /// What [`Salted`] mixes into its hash, which stands for a patch
        /// that changed the key's hash.
        static SALT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    /// A key whose hash a patch can change without changing its layout.
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Salted(u64);

    impl std::hash::Hash for Salted {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.hash(state);
            SALT.with(std::cell::Cell::get).hash(state);
        }
    }

    #[test]
    fn a_remount_finds_the_rows_after_the_patch_changed_the_keys_hash() {
        let build = || {
            use smudge_core::element::IntoElement;
            panel()
                .column()
                .child(
                    virtual_list_keyed(
                        || Rc::from((0..200).map(Salted).collect::<Vec<Salted>>()),
                        |item| *item,
                        100.0,
                        |item| panel().height(px(row_height(item.0))),
                    )
                    .height(px(480.0)),
                )
                .into_any()
        };
        let mut harness = Harness::new(640.0, 480.0, 1.0, build);
        settle(&mut harness);
        let before = visible_rows(&harness.layout_dump());
        SALT.with(|salt| salt.set(1));
        let mut kept = probes(true);
        kept.push(probe_of::<Salted>());
        kept.push(probe_of::<Vec<Salted>>());
        kept.push(probe_of::<Rc<RefCell<KeyedList<Salted, Salted>>>>());
        // SAFETY: the test never changes a type's layout, so every layout
        // it lists is the one the values hold.
        unsafe { smudge_core::hot::note_kept_types(kept.into_iter().collect()) };
        harness.app().remount(build);
        settle(&mut harness);
        assert_eq!(
            before,
            visible_rows(&harness.layout_dump()),
            "the rows were lost to the keys' new hash"
        );
    }

    #[test]
    fn a_remount_after_a_patch_keeps_the_rows_the_list_showed() {
        let (before, after) = remount_scrolled(probes(true));
        assert_eq!(before, after, "the list came back elsewhere");
    }

    #[test]
    fn the_offset_alone_brings_the_list_back_against_estimates() {
        // Without its measured heights the list re-aims the carried offset
        // against estimates, which is what carrying its state avoids.
        let (before, after) = remount_scrolled(probes(false));
        assert_ne!(before, after, "the measured heights were carried anyway");
    }
}
