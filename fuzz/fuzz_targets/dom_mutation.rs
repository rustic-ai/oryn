#![no_main]

use libfuzzer_sys::fuzz_target;
use oryn_native::dom::{DomArena, NodeKind};

fuzz_target!(|data: &[u8]| {
    let mut dom = DomArena::default();
    let root = dom.insert(NodeKind::Document);
    let mut nodes = vec![root];
    for (index, byte) in data.iter().copied().take(1024).enumerate() {
        match byte % 4 {
            0 => nodes.push(dom.insert(NodeKind::Text {
                data: format!("{index}:{byte}"),
            })),
            1 if nodes.len() > 1 => {
                let child = nodes[index % nodes.len()];
                let parent = nodes[usize::from(byte) % nodes.len()];
                let _ = dom.append_child(parent, child);
            }
            2 if nodes.len() > 1 => {
                let node = nodes[index % nodes.len()];
                let _ = dom.detach(node);
            }
            3 if nodes.len() > 1 => {
                let position = index % nodes.len();
                let node = nodes[position];
                if dom.remove(node).is_ok() {
                    nodes.swap_remove(position);
                }
            }
            _ => {}
        }
    }
});
