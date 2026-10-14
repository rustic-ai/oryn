// Tree completion staging uses the original prepared Stylo graph.
struct PreparedPendingImportCompletion {
    graph: oryn_layout::cssom::native_import_graph::PreparedImportCompletion,
    _snapshot_charge: std::sync::Arc<dyn Send + Sync>,
}
