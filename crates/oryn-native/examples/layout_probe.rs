use oryn_common::v3::ProjectionProfile;
use oryn_common::v4::{GeometryAvailability, ObservationOptions, ViewportState};
use oryn_native::{Browser, ContextOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = Browser::new().new_context_with_options(ContextOptions::default().with_viewport(
        ViewportState {
            width: 400.0,
            height: 300.0,
            device_pixel_ratio: 1.0,
        },
    ));
    let page = context.new_page();
    page.load_html(
        "https://layout.example.test/probe",
        r#"<!doctype html><style>
          body{margin:0}.block{width:200px;padding:10px;border:2px solid}
          .flex{display:flex;width:300px;gap:10px}.flex>button{width:80px;height:30px}
          .grid{display:grid;grid-template-columns:1fr 1fr;width:300px;gap:8px}
          .grid>span{height:20px}.text{width:90px;font-size:16px}
          .move{width:40px;height:20px;transform:translate(12px,8px)}
          .spacer{height:500px}.fixed{position:fixed;left:5px;top:7px;width:20px;height:20px}
        </style>
        <div id=block class=block>Block</div>
        <div class=flex><button id=f1>One</button><button id=f2>Two</button></div>
        <div class=grid><span id=grid-first>A</span><span id=grid-second>B</span><span id=grid-third>C</span></div>
        <p id=text class=text>Oryn deterministic inline wrapping fixture text</p>
        <div id=move class=move></div><div class=spacer></div><div id=fixed class=fixed></div>
        <table id=table><tr><td>Cell</td></tr></table>"#,
    )
    .await?;
    let observation = page
        .observe_with(ObservationOptions {
            profile: ProjectionProfile::Full,
            include_geometry: true,
            viewport_only: false,
            include_hidden: true,
        })
        .await?;
    let by_selector = |selector: &str| {
        observation
            .nodes
            .iter()
            .find(|node| node.semantic.selector.as_deref() == Some(selector))
            .and_then(|node| node.geometry.as_ref())
            .cloned()
            .unwrap_or_else(|| panic!("missing geometry for {selector}"))
    };
    let block = by_selector("#block");
    let f1 = by_selector("#f1");
    let f2 = by_selector("#f2");
    let grid_first = by_selector("#grid-first");
    let grid_second = by_selector("#grid-second");
    let grid_third = by_selector("#grid-third");
    let text = by_selector("#text");
    let moved = by_selector("#move");
    let fixed_before = by_selector("#fixed");
    let table = by_selector("#table");
    let scroll = page.scroll_to(0.0, 400.0).await?;
    let fixed_after = page
        .observe_with(ObservationOptions {
            profile: ProjectionProfile::Full,
            include_geometry: true,
            viewport_only: false,
            include_hidden: true,
        })
        .await?
        .nodes
        .into_iter()
        .find(|node| node.semantic.selector.as_deref() == Some("#fixed"))
        .and_then(|node| node.geometry)
        .expect("fixed geometry after scroll");

    let rect = |geometry: &oryn_common::v4::NodeGeometry| {
        geometry
            .box_model
            .as_ref()
            .map(|model| model.border_box)
            .expect("available border box")
    };
    let result = serde_json::json!({
        "schema_version": 1,
        "domain": "native",
        "viewport": observation.viewport,
        "relations": {
            "block_border_width": rect(&block).width,
            "flex_second_right_of_first": rect(&f2).x > rect(&f1).x,
            "grid_second_right_of_first": rect(&grid_second).x > rect(&grid_first).x,
            "grid_third_below_first": rect(&grid_third).y > rect(&grid_first).y,
            "text_has_fragments": !text.client_rects.is_empty(),
            "transform_moves_x": rect(&moved).x >= 12.0,
            "fixed_stable_after_scroll": (rect(&fixed_before).y - rect(&fixed_after).y).abs() < 0.01,
            "scroll_clamped": scroll.y > 0.0,
            "table_geometry_unavailable": table.availability == GeometryAvailability::Unsupported,
        },
        "outcome": {"status": "passed"}
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
