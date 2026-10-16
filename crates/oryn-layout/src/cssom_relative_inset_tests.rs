#[cfg(test)]
mod relative_resolved_inset_tests {
    use super::*;
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
