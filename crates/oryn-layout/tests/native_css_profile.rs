//! Admission must be deterministic before any native document is created.
use oryn_layout::cssom;
const BASE: &str = "https://example.test/native-profile.css";

#[test]
fn native_profile_admits_consumed_properties_before_first_document_on_all_threads() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads = (0..8)
        .map(|index| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                if index % 2 == 0 {
                    assert!(cssom::supports("display", Some("grid"), BASE));
                }
                let properties = cssom::supported_properties();
                for (property, value) in [
                    ("display", "grid"),
                    ("grid-template-columns", "1fr 2fr"),
                    ("column-count", "2"),
                    ("column-fill", "auto"),
                    ("column-rule", "2px solid red"),
                    ("writing-mode", "vertical-rl"),
                    ("font-variation-settings", "\"wght\" 650"),
                    ("container-type", "inline-size"),
                    ("container", "card / inline-size"),
                    ("break-before", "page"),
                    ("break-before", "recto"),
                    ("break-after", "column"),
                    ("break-inside", "avoid-column"),
                    ("page-break-before", "always"),
                    ("page", "chapter"),
                    ("ruby-align", "center"),
                    ("ruby-position", "under"),
                    ("text-combine-upright", "all"),
                    ("text-emphasis", "filled circle red"),
                    ("scroll-margin-block", "4px"),
                    ("scroll-padding", "5px"),
                    ("scrollbar-gutter", "stable both-edges"),
                    ("overscroll-behavior", "contain"),
                    ("accent-color", "red"),
                    ("field-sizing", "content"),
                    ("clip-rule", "evenodd"),
                    ("vector-effect", "non-scaling-stroke"),
                ] {
                    assert!(
                        properties.iter().any(|(name, _)| name == property),
                        "missing {property}"
                    );
                    assert!(
                        cssom::supports(property, Some(value), BASE),
                        "unsupported {property}: {value}"
                    );
                    let declaration =
                        cssom::mutate_declaration("", property, value, "", BASE, false).unwrap();
                    assert!(
                        declaration
                            .css_text
                            .as_ref()
                            .is_some_and(|text| !text.is_empty()),
                        "discarded {property}: {value}"
                    );
                }
                assert!(!cssom::supports("page-break-before", Some("recto"), BASE));
                for (property, value) in [
                    ("size", "A4 landscape"),
                    ("page-orientation", "rotate-left"),
                ] {
                    let declaration = cssom::read_page_declaration(
                        &format!("{property}:{value}"),
                        property,
                        BASE,
                        false,
                    )
                    .unwrap();
                    assert!(
                        !declaration.value.is_empty(),
                        "discarded page descriptor {property}"
                    );
                }
                // Native admission must not turn on the blanket experimental gate.
                assert!(!cssom::supports("corner-shape", Some("scoop"), BASE));
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn native_content_visibility_preserves_special_hidden_interpolation() {
    use style::values::animated::{Animate, Procedure, ToAnimatedZero};
    use style::values::distance::{ComputeSquaredDistance, SquaredDistance};
    use style::values::specified::ContentVisibility::{Auto, Hidden, Visible};
    for visible in [Auto, Visible] {
        for progress in [0.0, 0.001, 0.5, 0.999, 1.0] {
            let procedure = Procedure::Interpolate { progress };
            assert_eq!(
                Hidden.animate(&visible, procedure),
                Ok(if progress > 0.0 { visible } else { Hidden })
            );
            assert_eq!(
                visible.animate(&Hidden, procedure),
                Ok(if progress < 1.0 { visible } else { Hidden })
            );
        }
        assert_eq!(visible.animate(&Hidden, Procedure::Add), Err(()));
        assert_eq!(
            visible.animate(&Hidden, Procedure::Accumulate { count: 2 }),
            Err(())
        );
        assert_eq!(visible.to_animated_zero(), Err(()));
        assert_eq!(
            visible.compute_squared_distance(&Hidden),
            Ok(SquaredDistance::from_sqrt(1.0))
        );
        assert_eq!(
            visible.compute_squared_distance(&visible),
            Ok(SquaredDistance::from_sqrt(0.0))
        );
    }
    // Non-hidden pairs use the engine's ordinary discrete fallback.
    assert_eq!(
        Auto.animate(&Visible, Procedure::Interpolate { progress: 0.5 }),
        Err(())
    );
}

#[test]
fn native_font_synthesis_style_roundtrips_all_serialized_variants() {
    use style::values::specified::FontSynthesisStyle;
    for value in [
        FontSynthesisStyle::Auto,
        FontSynthesisStyle::None,
        FontSynthesisStyle::ObliqueOnly,
    ] {
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(
            serde_json::from_str::<FontSynthesisStyle>(&encoded).unwrap(),
            value
        );
    }
}

#[test]
fn container_shorthand_serializes_optional_normal_and_non_normal_types() {
    for (text, expected) in [
        ("container-name:card;container-type:normal", "card"),
        (
            "container-name:card;container-type:inline-size",
            "card / inline-size",
        ),
    ] {
        assert_eq!(
            cssom::read_declaration(text, "container", BASE, false)
                .unwrap()
                .value,
            expected
        );
    }
}

#[test]
fn native_display_zoom_transform_and_tree_abiding_selector_admission() {
    for value in [
        "ruby",
        "block ruby",
        "ruby-base",
        "ruby-base-container",
        "ruby-text",
        "ruby-text-container",
    ] {
        assert!(
            cssom::supports("display", Some(value), BASE),
            "display: {value}"
        );
        let view =
            cssom::read_declaration(&format!("display:{value}"), "display", BASE, false).unwrap();
        assert_eq!(view.value, value);
    }
    for value in [".1", "2", "normal"] {
        assert!(cssom::supports("zoom", Some(value), BASE));
    }
    for value in [
        "content-box",
        "border-box",
        "fill-box",
        "stroke-box",
        "view-box",
    ] {
        assert!(
            cssom::supports("transform-box", Some(value), BASE),
            "transform-box: {value}"
        );
    }
    for selector in [
        ":in-range",
        ":out-of-range",
        "::slotted(span)::before",
        "::slotted(span)::after",
        "div::before::marker",
    ] {
        assert!(
            cssom::supports(&format!("selector({selector})"), None, BASE),
            "{selector}"
        );
    }
    assert!(!cssom::supports(
        "selector(::slotted(span)::first-line)",
        None,
        BASE
    ));
}

#[test]
fn ruby_adapter_retains_measurable_boxes_and_normal_generation() {
    use style::values::specified::box_::Display;
    for display in [
        Display::Ruby,
        Display::RubyBase,
        Display::RubyText,
        Display::RubyBaseContainer,
        Display::RubyTextContainer,
    ] {
        assert_eq!(
            stylo_taffy::convert::display(display),
            taffy::Display::Block
        );
        assert_eq!(
            stylo_taffy::convert::box_generation_mode(display),
            taffy::BoxGenerationMode::Normal
        );
    }
    assert_eq!(
        stylo_taffy::convert::display(Display::None),
        taffy::Display::None
    );
    assert_eq!(
        stylo_taffy::convert::box_generation_mode(Display::None),
        taffy::BoxGenerationMode::None
    );
}

#[test]
fn native_generated_content_admits_image_speech_alternatives() {
    let value = r#"url(icons/check.svg) / "Selected""#;
    assert!(cssom::supports("content", Some(value), BASE));
    let declaration =
        cssom::read_declaration(&format!("content:{value}"), "content", BASE, false).unwrap();
    assert!(declaration.value.contains("Selected"));
    assert!(declaration.value.contains('/'));
}

#[test]
fn sideways_values_retain_native_inline_and_block_axes() {
    use style::logical_geometry::{WritingMode, WritingModeProperty};
    use style::properties::longhands::direction::computed_value::T as Direction;
    use style::properties::{ComputedValues, style_structs::Font};
    let initial = ComputedValues::initial_values_with_font_override(Font::initial_values());
    for (value, mode, leftward_block) in [
        ("sideways-rl", WritingModeProperty::SidewaysRl, true),
        ("sideways-lr", WritingModeProperty::SidewaysLr, false),
    ] {
        assert!(cssom::supports("writing-mode", Some(value), BASE));
        assert_eq!(
            cssom::read_declaration(
                &format!("writing-mode:{value}"),
                "writing-mode",
                BASE,
                false
            )
            .unwrap()
            .value,
            value
        );
        for direction in [Direction::Ltr, Direction::Rtl] {
            let mut inherited = initial.get_inherited_box().clone();
            inherited.set_writing_mode(mode);
            inherited.set_direction(direction);
            let axes = WritingMode::new(&inherited);
            assert!(axes.is_vertical() && axes.is_sideways());
            assert_eq!(axes.is_vertical_rl(), leftward_block);
            assert_eq!(
                axes.is_inline_tb(),
                if leftward_block {
                    direction == Direction::Ltr
                } else {
                    direction == Direction::Rtl
                }
            );
        }
    }
    assert!(!cssom::supports(
        "writing-mode",
        Some("sideways-diagonal"),
        BASE
    ));
    assert!(cssom::supports(
        "selector(x-host::part(label):dir(rtl))",
        None,
        BASE
    ));
    assert!(!cssom::supports(
        "selector(x-host::part(label):first-child)",
        None,
        BASE
    ));
}

fn assert_fragmentation_typed_number(values: style::typed_om::TypedValueList, expected: f32) {
    use style::typed_om::{NumericValue, TypedValue};
    let [TypedValue::Numeric(NumericValue::Unit(unit))] = values.values.as_slice() else {
        panic!("expected one numeric unit value, got {values:?}");
    };
    assert_eq!(unit.unit_str(), "number");
    assert_eq!(unit.value, expected);
}

#[test]
fn native_fragmentation_limits_reify_specified_positive_integers() {
    use style::{
        context::QuirksMode,
        properties::{LonghandId, PropertyDeclarationId, parse_style_attribute},
        stylesheets::{CssRuleType, UrlExtraData},
        typed_om::{MathValue, NumericValue, TypedValue},
    };
    let url_data = UrlExtraData::from(url::Url::parse(BASE).unwrap());
    for (name, id) in [
        ("orphans", LonghandId::Orphans),
        ("widows", LonghandId::Widows),
    ] {
        for (text, expected) in [("1", 1.0), ("2", 2.0), ("37", 37.0), ("+5", 5.0)] {
            let block = parse_style_attribute(
                &format!("{name}:{text}"),
                &url_data,
                None,
                QuirksMode::NoQuirks,
                CssRuleType::Style,
            );
            let (declaration, _) = block
                .get(PropertyDeclarationId::Longhand(id))
                .expect("positive integer declaration");
            // Exercise the generated PropertyDeclaration dispatch, not just
            // an unwrapped integer's conversion.
            assert_fragmentation_typed_number(
                declaration
                    .to_typed_value_list()
                    .expect("specified Typed OM"),
                expected,
            );
        }
        for invalid in ["0", "-1", "1.5", "2px", "auto"] {
            let block = parse_style_attribute(
                &format!("{name}:{invalid}"),
                &url_data,
                None,
                QuirksMode::NoQuirks,
                CssRuleType::Style,
            );
            assert!(
                block.get(PropertyDeclarationId::Longhand(id)).is_none(),
                "invalid {name}:{invalid} must not become a typed declaration"
            );
        }

        // Specified calc values retain the pinned engine's CSSMathSum shape;
        // the positive wrapper must delegate without discarding that shape.
        let block = parse_style_attribute(
            &format!("{name}:calc(1 + 2)"),
            &url_data,
            None,
            QuirksMode::NoQuirks,
            CssRuleType::Style,
        );
        let (declaration, _) = block
            .get(PropertyDeclarationId::Longhand(id))
            .expect("positive integer calc declaration");
        let values = declaration.to_typed_value_list().expect("calc Typed OM");
        let [TypedValue::Numeric(NumericValue::Math(MathValue::Sum(sum)))] =
            values.values.as_slice()
        else {
            panic!("expected the specified calc sum, got {values:?}");
        };
        let [NumericValue::Unit(unit)] = sum.values.as_slice() else {
            panic!("expected the simplified numeric term, got {sum:?}");
        };
        assert_eq!(unit.unit_str(), "number");
        assert_eq!(unit.value, 3.0);
    }
}

#[test]
fn native_fragmentation_limits_reify_computed_positive_integers() {
    use style::properties::{ComputedValues, LonghandId, style_structs::Font};
    let initial = ComputedValues::initial_values_with_font_override(Font::initial_values());
    for id in [LonghandId::Orphans, LonghandId::Widows] {
        assert_fragmentation_typed_number(
            initial
                .property_value_to_typed_value_list(id)
                .expect("initial Typed OM"),
            2.0,
        );
    }
    let mut computed = (*initial).clone();
    for (orphans, widows) in [(1, 37), (9, 1)] {
        computed.mutate_inherited_box().set_orphans(orphans.into());
        computed.mutate_inherited_box().set_widows(widows.into());
        // These calls exercise the generated computed-property dispatch that
        // uses GreaterThanOrEqualToOne<i32>, independently of specified values.
        for (id, expected) in [(LonghandId::Orphans, orphans), (LonghandId::Widows, widows)] {
            assert_fragmentation_typed_number(
                computed
                    .property_value_to_typed_value_list(id)
                    .expect("computed Typed OM"),
                expected as f32,
            );
            assert_fragmentation_typed_number(
                initial
                    .property_value_to_typed_value_list(id)
                    .expect("unchanged initial Typed OM"),
                2.0,
            );
        }
    }
}
