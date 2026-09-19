//! Regression sources for the existing native top-layer layout-root producer.
use super::*;
#[test]
fn promoted_box_uses_initial_containing_block_and_keeps_only_its_own_clip() {
    // Pinned Core top-layer-containing-block.html and top-layer-parent-transform.html.
    for kind in [
        oryn_common::v4::TopLayerKind::ModalDialog,
        oryn_common::v4::TopLayerKind::Popover,
        oryn_common::v4::TopLayerKind::Fullscreen,
    ] {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[
   {"id":1,"parent":null,"children":[2],"tag":"body","attributes":{"style":"margin:0"}},
   {"id":2,"parent":1,"children":[3],"tag":"div","attributes":{"style":"position:absolute;left:1000px;top:1000px;width:20px;height:20px;transform:translate(100px,100px);overflow:hidden;clip-path:circle(1%);opacity:.3"}},
   {"id":3,"parent":2,"children":[4],"tag":"div","attributes":{"style":"position:absolute;left:0;top:20px;width:90%;height:100px;box-sizing:border-box;margin:0;padding:0;border:0;clip-path:inset(5px)"}},
   {"id":4,"parent":3,"tag":"div","attributes":{"style":"width:10px;height:10px"}}
  ],"stylesheets":[],"viewport":{"width":500,"height":300,"device_pixel_ratio":1},"scroll":ScrollState::default()})).unwrap();
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            node_id: 3,
            kind,
            anchor_source: None,
            picker: None,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let model = output.geometry[&3].box_model.as_ref().unwrap();
        assert!((model.border_box.x - 0.).abs() < 0.01);
        assert!((model.border_box.y - 20.).abs() < 0.01);
        assert!((model.border_box.width - 450.).abs() < 0.01);
        for id in [3, 4] {
            let clips = &output.paint_projections[&id].clips;
            assert!(!clips.iter().any(|clip| clip.owner == Some(2)));
            assert!(clips.iter().any(|clip| clip.owner == Some(3)));
        }
    }
}
