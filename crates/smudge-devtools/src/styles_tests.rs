use smudge_core::geometry::Color;
use smudge_core::style::{Align, Edges, Length, Style};
use smudge_core::theme::{ColorToken, CornerRadii};

use super::style_lines;

#[test]
fn only_the_properties_that_differ_from_the_default_are_listed() {
    assert!(style_lines(&Style::default()).is_empty());

    let mut style = Style::default();
    style.layout.width = Length::Px(120.0);
    style.layout.height = Length::Percent(50.0);
    style.layout.padding = Edges {
        top: Length::Px(4.0),
        right: Length::Px(8.0),
        bottom: Length::Px(4.0),
        left: Length::Px(8.0),
    };
    style.layout.border_width = Edges::all(1.0);
    style.layout.align_self = Some(Align::Center);
    style.paint.background = Some(Color::rgba(16, 32, 48, 128).into());
    style.paint.text_color = Some(ColorToken::ACCENT.into());
    style.paint.underline = true;
    style.paint.corner_radius = CornerRadii::all(6.0);
    assert_eq!(
        style_lines(&style),
        [
            ("width", String::from("120px")),
            ("height", String::from("50%")),
            ("padding", String::from("4px 8px")),
            ("align self", String::from("Center")),
            ("border", String::from("1px")),
            ("background", String::from("#10203080")),
            ("text color", String::from("theme accent")),
            ("underline", String::from("on")),
            ("radius", String::from("6px")),
        ]
    );
}
