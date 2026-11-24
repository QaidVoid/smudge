use crate::geometry::Size;
use crate::style::{Align, Direction, Display, Edges, LayoutProps, Length, Track, TrackSize};

#[test]
fn to_taffy_converts_px_percent_and_auto() {
    let props = LayoutProps {
        width: Length::pct(50.0),
        height: Length::px(30.0),
        min_width: Length::pct(25.0),
        max_height: Length::px(120.0),
        padding: Edges::xy(Length::px(4.0), Length::pct(10.0)),
        margin: Edges::all(Length::auto()),
        ..LayoutProps::default()
    };

    let style = props.to_taffy();

    assert_eq!(
        style.size.width.expand(),
        taffy::ExpandedDimension::Percent(0.5)
    );
    assert_eq!(
        style.size.height.expand(),
        taffy::ExpandedDimension::Length(30.0)
    );
    assert_eq!(
        style.min_size.width.expand(),
        taffy::ExpandedLengthPercentageAuto::Percent(0.25)
    );
    assert_eq!(
        style.max_size.height.expand(),
        taffy::ExpandedLengthPercentageAuto::Length(120.0)
    );
    assert_eq!(
        style.padding.left.expand(),
        taffy::ExpandedLengthPercentage::Length(4.0)
    );
    assert_eq!(
        style.padding.right.expand(),
        taffy::ExpandedLengthPercentage::Length(4.0)
    );
    assert_eq!(
        style.padding.top.expand(),
        taffy::ExpandedLengthPercentage::Percent(0.1)
    );
    assert_eq!(
        style.padding.bottom.expand(),
        taffy::ExpandedLengthPercentage::Percent(0.1)
    );
    assert_eq!(
        style.margin.left.expand(),
        taffy::ExpandedLengthPercentageAuto::Auto
    );
}

#[test]
fn default_layout_props_map_to_a_flex_row() {
    let style = LayoutProps::default().to_taffy();

    assert_eq!(style.display, taffy::Display::Flex);
    assert_eq!(style.flex_direction, taffy::FlexDirection::Row);
    assert_eq!(style.flex_wrap, taffy::FlexWrap::NoWrap);
    assert_eq!(style.flex_grow, 0.0);
    assert_eq!(style.flex_shrink, 1.0);
    assert_eq!(style.size.width.expand(), taffy::ExpandedDimension::Auto);
    assert_eq!(style.size.height.expand(), taffy::ExpandedDimension::Auto);
    assert_eq!(style.overflow.x, taffy::Overflow::Visible);
    assert_eq!(style.overflow.y, taffy::Overflow::Visible);
}

#[test]
fn automatic_padding_becomes_zero_and_gap_maps_to_lengths() {
    let props = LayoutProps {
        padding: Edges::all(Length::auto()),
        gap: Size {
            width: 7.0,
            height: 3.0,
        },
        ..LayoutProps::default()
    };

    let style = props.to_taffy();

    assert_eq!(
        style.padding.left.expand(),
        taffy::ExpandedLengthPercentage::Length(0.0)
    );
    assert_eq!(
        style.gap.width.expand(),
        taffy::ExpandedLengthPercentage::Length(7.0)
    );
    assert_eq!(
        style.gap.height.expand(),
        taffy::ExpandedLengthPercentage::Length(3.0)
    );
}

#[test]
fn a_row_with_padding_and_gap_lays_out_its_children() {
    let mut tree: taffy::TaffyTree<()> = taffy::TaffyTree::new();

    let container = LayoutProps {
        direction: Direction::Row,
        width: Length::px(100.0),
        height: Length::px(40.0),
        padding: Edges::all(Length::px(10.0)),
        gap: Size {
            width: 5.0,
            height: 0.0,
        },
        align_items: Some(Align::Start),
        ..LayoutProps::default()
    };
    let fixed = LayoutProps {
        width: Length::px(20.0),
        height: Length::px(10.0),
        ..LayoutProps::default()
    };
    let half = LayoutProps {
        width: Length::pct(50.0),
        height: Length::px(10.0),
        ..LayoutProps::default()
    };

    let fixed_id = tree.new_leaf(fixed.to_taffy()).unwrap();
    let half_id = tree.new_leaf(half.to_taffy()).unwrap();
    let root = tree
        .new_with_children(container.to_taffy(), &[fixed_id, half_id])
        .unwrap();
    tree.compute_layout(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::Definite(100.0),
            height: taffy::AvailableSpace::Definite(40.0),
        },
    )
    .unwrap();

    let fixed_layout = tree.layout(fixed_id).unwrap();
    assert_eq!(fixed_layout.location, taffy::Point { x: 10.0, y: 10.0 });
    assert_eq!(
        fixed_layout.size,
        taffy::Size {
            width: 20.0,
            height: 10.0,
        }
    );

    let half_layout = tree.layout(half_id).unwrap();
    assert_eq!(half_layout.location, taffy::Point { x: 35.0, y: 10.0 });
    assert_eq!(
        half_layout.size,
        taffy::Size {
            width: 40.0,
            height: 10.0,
        }
    );

    let root_layout = tree.layout(root).unwrap();
    assert_eq!(
        root_layout.size,
        taffy::Size {
            width: 100.0,
            height: 40.0,
        }
    );
}

#[test]
fn hiding_keeps_the_display_so_showing_restores_it() {
    let hidden_block = LayoutProps {
        display: Display::Block,
        hidden: true,
        ..LayoutProps::default()
    };
    assert_eq!(hidden_block.to_taffy().display, taffy::Display::None);

    let shown = LayoutProps {
        hidden: false,
        ..hidden_block
    };
    assert_eq!(shown.to_taffy().display, taffy::Display::Block);
}

/// Reads the one sizing function of a template component, which the tests
/// here always write.
fn single(component: &taffy::GridTemplateComponent<String>) -> taffy::TrackSizingFunction {
    match component {
        taffy::GridTemplateComponent::Single(sizing) => *sizing,
        taffy::GridTemplateComponent::Repeat(_) => {
            panic!("the templates the tests write hold no repeat")
        }
    }
}

#[cfg(feature = "grid")]
#[test]
fn to_taffy_converts_a_grid_templates_tracks_and_spans() {
    let props = LayoutProps {
        display: Display::Grid,
        grid_columns: vec![
            Track::Px(90.0),
            Track::Fr(1.0),
            Track::MinMax(TrackSize::Px(40.0), TrackSize::Fr(2.0)),
            Track::MinContent,
            Track::Px(f32::NAN),
        ],
        grid_rows: vec![Track::MaxContent],
        column_span: 3,
        row_span: 2,
        ..LayoutProps::default()
    };

    let style = props.to_taffy();

    assert_eq!(style.display, taffy::Display::Grid);
    let columns: Vec<_> = style.grid_template_columns.iter().map(single).collect();
    assert_eq!(
        columns[0].min.expand(),
        taffy::ExpandedMinTrackSizingFunction::Length(90.0)
    );
    assert_eq!(
        columns[0].max.expand(),
        taffy::ExpandedMaxTrackSizingFunction::Length(90.0)
    );
    assert_eq!(
        columns[1].min.expand(),
        taffy::ExpandedMinTrackSizingFunction::Auto,
        "a share of the leftover space accepts what the content needs"
    );
    assert_eq!(
        columns[1].max.expand(),
        taffy::ExpandedMaxTrackSizingFunction::Fr(1.0)
    );
    assert_eq!(
        columns[2].min.expand(),
        taffy::ExpandedMinTrackSizingFunction::Length(40.0)
    );
    assert_eq!(
        columns[2].max.expand(),
        taffy::ExpandedMaxTrackSizingFunction::Fr(2.0)
    );
    assert_eq!(
        columns[3].min.expand(),
        taffy::ExpandedMinTrackSizingFunction::MinContent
    );
    assert_eq!(
        columns[3].max.expand(),
        taffy::ExpandedMaxTrackSizingFunction::MinContent
    );
    assert_eq!(
        columns[4].min.expand(),
        taffy::ExpandedMinTrackSizingFunction::Length(0.0),
        "a track size that is not a number is zero"
    );
    assert_eq!(
        single(&style.grid_template_rows[0]).max.expand(),
        taffy::ExpandedMaxTrackSizingFunction::MaxContent
    );
    assert_eq!(
        style.grid_column.start,
        taffy::GridPlacement::Span(3),
        "the child reaches across three columns"
    );
    assert_eq!(style.grid_column.end, taffy::GridPlacement::Auto);
    assert_eq!(style.grid_row.start, taffy::GridPlacement::Span(2));
}

#[cfg(feature = "grid")]
#[test]
fn a_child_that_spans_one_track_keeps_the_engine_s_default() {
    let props = LayoutProps {
        display: Display::Grid,
        ..LayoutProps::default()
    };

    let style = props.to_taffy();

    assert_eq!(style.grid_column.start, taffy::GridPlacement::Auto);
    assert_eq!(style.grid_row.start, taffy::GridPlacement::Auto);
    assert!(style.grid_template_columns.is_empty());
}
