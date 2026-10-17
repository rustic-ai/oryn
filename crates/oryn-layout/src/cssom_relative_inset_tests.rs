#[cfg(test)]
mod relative_resolved_inset_tests {
    use super::*;
    #[test]
    fn positioned_cssom_insets_use_padding_cb_without_changing_authored_ends() {
        for position in ["absolute", "fixed"] {
            for wm in ["horizontal-tb", "vertical-lr", "vertical-rl"] {
                for direction in ["ltr", "rtl"] {
                    for (rule, expected) in [
                        ("top:10%;bottom:50%;left:25%;right:75%;height:0;width:0", [20., 100., 100., 300.]),
                        ("top:calc(10% - 1px);bottom:calc(50% - 3px);left:calc(25% - 2px);right:calc(75% - 4px);height:0;width:0", [19., 97., 98., 296.]),
                        ("top:1px;bottom:3px;left:2px;right:4px;height:0;width:0;margin:auto", [1., 3., 2., 4.]),
                        ("top:auto;bottom:3px;left:auto;right:4px;height:0;width:0", [197., 3., 396., 4.]),
                        ("top:1px;bottom:auto;left:2px;right:auto;height:0;width:0", [1., 199., 2., 398.]),
                    ] {
                        let mut input = tests::column_fixture("", vec![tests::node(2, Some(1), "div", "")]);
                        input.nodes[0].tag = "section".into();
                        input.stylesheets = vec![format!("section{{position:relative;transform:scale(1);width:368px;height:184px;padding:8px 16px;border:16px solid;margin:32px;writing-mode:{wm};direction:{direction};visibility:hidden}}div{{position:{position};{rule}}}")];
                        let output = compute(&input, &LayoutLimits::default()).unwrap();
                        for (property, value) in ["top", "bottom", "left", "right"].into_iter().zip(expected) {
                            assert_eq!(output.resolved_values[&2][property], format!("{value}px"), "{position}/{wm}/{direction}/{rule}/{property}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn stationary_positioned_box_still_publishes_used_insets() {
        let mut input = tests::column_fixture("", vec![tests::node(2, Some(1), "div", "")]);
        input.nodes[0].tag = "section".into();
        input.stylesheets = vec!["section{position:relative;width:200px;height:100px}div{position:absolute;top:0;left:0;width:20px;height:20px}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (property, expected) in [("top", "0px"), ("left", "0px"), ("bottom", "80px"), ("right", "180px")] {
            assert_eq!(output.resolved_values[&2][property], expected);
        }
    }

    #[test]
    fn auto_insets_exclude_border_box_and_nonzero_margins() {
        let mut input = tests::column_fixture("", vec![tests::node(2, Some(1), "div", "")]);
        input.nodes[0].tag = "section".into();
        input.stylesheets = vec!["section{position:relative;width:200px;height:100px}div{position:absolute;top:1px;left:2px;width:20px;height:10px;padding:2px;border:3px solid;margin:4px}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.resolved_values[&2]["bottom"], "71px");
        assert_eq!(output.resolved_values[&2]["right"], "160px");
    }

    #[test]
    fn non_positioned_and_boxless_insets_are_not_projected() {
        for rule in ["position:static", "position:sticky", "position:absolute;display:none", "position:fixed;display:contents"] {
            let mut input = tests::column_fixture("", vec![tests::node(2, Some(1), "div", "")]);
            input.stylesheets = vec![format!("div{{{rule};top:10%;bottom:calc(50% - 3px)}}")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert!(output.resolved_values.get(&2).is_none_or(|values| !values.contains_key("top") && !values.contains_key("bottom")), "{rule}");
        }
    }
    #[test]
    fn relative_indefinite_height_percentages_remain_auto_without_viewport_basis() {
        let style=BTreeMap::from([("top".into(),"calc(10% - 1px)".into()),
            ("bottom".into(),"3px".into()),("left".into(),"25%".into())]);
        assert_eq!(relative_inset_lengths(&style,200.,None,16.,ViewportState::default()),
            [Some(50.),None,None,Some(3.)]);
    }
    #[test]
    fn relative_used_insets_reuse_layout_offsets_and_do_not_preserve_auto() {
        for (rule, expected) in [
            ("top:10%;left:10%", [20., -20., 10., -10.]),
            ("top:calc(10% - 1px);left:calc(10% - 1px)", [19., -19., 9., -9.]),
            ("right:4px;bottom:3px", [-4., 4., -3., 3.]),
            ("top:auto;right:auto;bottom:auto;left:auto", [0., 0., 0., 0.]),
            ("top:10%;bottom:50%;left:25%;right:75%", [50., 150., 10., 50.]),
            ("direction:rtl;top:calc(10% - 1px);bottom:calc(50% - 3px);left:calc(25% - 2px);right:calc(75% - 4px)", [48., 146., 9., 47.]),
        ] {
            let mut input = tests::column_fixture("", vec![tests::node(2, Some(1), "div", "")]);
            input.nodes[0].tag = "section".into();
            input.stylesheets = vec![format!("section{{width:200px;height:100px}}div{{position:relative;width:20px;height:20px;{rule}}}")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            for (property, value) in ["left", "right", "top", "bottom"].into_iter().zip(expected) {
                assert_eq!(output.resolved_values[&2][property], format!("{value}px"), "{rule}: {property}");
            }
        }
    }
}
