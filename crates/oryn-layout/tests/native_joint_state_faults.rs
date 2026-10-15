// Included within the owning module.
// Included within the owning module.
use oryn_dom::{DomRuntime, StylesheetMetadataPreparationError};
use oryn_layout::cssom::{ImportAdmission, ImportGraph, MediaListEdit, new_native_style_lock};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Admission {
    fail: AtomicBool,
}
impl ImportAdmission for Admission {
    fn checkpoint(&self) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            Err("forced native admission failure".into())
        } else {
            Ok(())
        }
    }
    fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
        self.checkpoint()?;
        Ok(Arc::new(()))
    }
}
fn fixture() -> (DomRuntime, oryn_dom::NodeId, ImportGraph, Admission) {
    let mut dom = DomRuntime::new();
    let node = dom.create_element("style");
    dom.append_child(dom.document(), node).unwrap();
    dom.set_stylesheet_text(node, "div { color: red }".into())
        .unwrap();
    dom.set_stylesheet_media(node, "screen".into()).unwrap();
    dom.set_stylesheet_disabled(node, false).unwrap();
    let admission = Admission::default();
    let graph = ImportGraph::parse(
        "div { color: red }",
        "https://example.test/root.css",
        false,
        8,
        new_native_style_lock(),
        &admission,
    )
    .unwrap();
    graph
        .prepare_sheet_media(&graph.root(), MediaListEdit::Replace("screen"), &admission)
        .unwrap()
        .commit();
    (dom, node, graph, admission)
}

#[test]
fn native_prepare_admission_failure_preserves_dom_and_native() {
    let (dom, node, graph, admission) = fixture();
    let stamp = dom.stylesheet_journal_stamp();
    admission.fail.store(true, Ordering::SeqCst);
    assert!(
        graph
            .prepare_sheet_media(&graph.root(), MediaListEdit::Replace("print"), &admission)
            .is_err()
    );
    assert!(
        graph
            .prepare_sheet_disabled(&graph.root(), true, &admission)
            .is_err()
    );
    assert_eq!(graph.root().media_text(), "screen");
    assert!(!graph.root().disabled());
    assert_eq!(dom.arena().stylesheet_media(node).unwrap(), "screen");
    assert_eq!(dom.stylesheet_journal_stamp(), stamp);
}

#[test]
fn stale_final_dom_stamp_cancels_prepared_native_publication() {
    let (mut dom, node, graph, admission) = fixture();
    let native = graph
        .prepare_sheet_media(&graph.root(), MediaListEdit::Replace("print"), &admission)
        .unwrap();
    let stamp = dom.stylesheet_journal_stamp();
    let bytes = stamp.requested_bytes().unwrap();
    let buffers = stamp
        .prepare_buffers(admission.reserve(bytes).unwrap(), bytes)
        .unwrap();
    // A real metadata mutation between snapshot and final borrow invalidates
    // the stamp. It is not rolled back; the proposed paired edit is canceled.
    dom.set_stylesheet_disabled(node, false).unwrap();
    let current = dom.stylesheet_journal_stamp();
    assert!(matches!(
        dom.prepare_stylesheet_metadata_replacement(node, None, Some(native.media_text()), buffers),
        Err(StylesheetMetadataPreparationError::Stale)
    ));
    drop(native);
    assert_eq!(graph.root().media_text(), "screen");
    assert_eq!(dom.arena().stylesheet_media(node).unwrap(), "screen");
    assert_eq!(dom.stylesheet_journal_stamp(), current);
}

#[test]
fn admitted_pair_commits_media_and_disabled_without_owner_attributes() {
    let (mut dom, node, graph, admission) = fixture();
    let native = graph
        .prepare_sheet_media(&graph.root(), MediaListEdit::Replace("print"), &admission)
        .unwrap();
    let stamp = dom.stylesheet_journal_stamp();
    let bytes = stamp.requested_bytes().unwrap();
    let buffers = stamp
        .prepare_buffers(admission.reserve(bytes).unwrap(), bytes)
        .unwrap();
    let target = dom
        .prepare_stylesheet_metadata_replacement(node, None, Some(native.media_text()), buffers)
        .unwrap_or_else(|_| panic!("valid DOM preparation"));
    native.commit();
    target.commit();
    assert_eq!(graph.root().media_text(), "print");
    assert_eq!(dom.arena().stylesheet_media(node).unwrap(), "print");
    let native = graph
        .prepare_sheet_disabled(&graph.root(), true, &admission)
        .unwrap();
    let stamp = dom.stylesheet_journal_stamp();
    let bytes = stamp.requested_bytes().unwrap();
    let buffers = stamp
        .prepare_buffers(admission.reserve(bytes).unwrap(), bytes)
        .unwrap();
    let target = dom
        .prepare_stylesheet_metadata_replacement(node, Some(true), None, buffers)
        .unwrap_or_else(|_| panic!("valid DOM preparation"));
    native.commit();
    target.commit();
    assert!(graph.root().disabled());
    assert_eq!(
        dom.arena().get(node).unwrap().stylesheet_disabled,
        Some(true)
    );
    assert!(
        dom.arena()
            .get(node)
            .unwrap()
            .attributes
            .get_null("disabled")
            .is_none()
    );
    assert!(
        dom.arena()
            .get(node)
            .unwrap()
            .attributes
            .get_null("media")
            .is_none()
    );
}
