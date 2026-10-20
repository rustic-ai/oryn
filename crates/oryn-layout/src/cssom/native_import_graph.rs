// Included within owning module.
// Included within owning module.
use super::{RuleMutationError, RuleView, rule_view};
use std::{
    cell::RefCell,
    sync::{Arc as LeaseArc, Mutex},
};
use style::{
    context::QuirksMode,
    media_queries::MediaList,
    servo_arc::Arc,
    shared_lock::{Locked, SharedRwLock},
    stylesheets::{
        AllowImportRules, CssRule, Origin, Stylesheet, StylesheetInDocument, StylesheetLoader,
        UrlExtraData,
        import_rule::{ImportLayer, ImportRule, ImportSheet, ImportSupportsCondition},
    },
};

/// Implemented by the native resource budget, never by authored code.
/// Reservations must release their charge on Drop; the graph retains them for
/// as long as any exported native sheet/rule handle is alive.
pub trait ImportAdmission {
    fn checkpoint(&self) -> Result<(), String>;
    fn reserve(&self, bytes: usize) -> Result<LeaseArc<dyn Send + Sync>, String>;
}
type Leases = LeaseArc<Mutex<Vec<LeaseArc<dyn Send + Sync>>>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OccurrenceId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportState {
    Pending,
    Refused,
    Complete,
    Cancelled,
    Detached,
}
#[derive(Clone)]
pub struct NativeSheet {
    sheet: Arc<Stylesheet>,
    leases: Leases,
}
/// Retains the exact typed Stylo rule and owner allocation ledger. Snapshot
/// RuleView is derived on demand; paths are used only to acquire this handle.
#[derive(Clone)]
struct RuleAncestor {
    rule: CssRule,
    list: Arc<Locked<style::stylesheets::CssRules>>,
    containing: style::stylesheets::CssRuleTypes,
    relative: Option<style::stylesheets::CssRuleType>,
}
#[derive(Clone)]
pub struct NativeRule {
    rule: CssRule,
    owner: NativeSheet,
    containing: style::stylesheets::CssRuleTypes,
    relative: Option<style::stylesheets::CssRuleType>,
    list: Arc<Locked<style::stylesheets::CssRules>>,
    ancestors: LeaseArc<Vec<RuleAncestor>>,
    ancestor_len: usize,
    capture_charge: LeaseArc<dyn Send + Sync>,
}
impl NativeRule {
    pub fn layer_name_count(&self) -> Option<usize> {
        match &self.rule {
            CssRule::LayerStatement(rule) => Some(rule.names.len()),
            _ => None,
        }
    }
    pub fn layer_name_at(&self, index: usize) -> Option<String> {
        use style_traits::ToCss;
        match &self.rule {
            CssRule::LayerStatement(rule) => rule.names.get(index).map(|name| name.to_css_string()),
            _ => None,
        }
    }
    fn member(&self) -> bool {
        let guard = self.owner.sheet.shared_lock.read();
        self.list
            .read_with(&guard)
            .0
            .iter()
            .any(|rule| Self::same_native_rule(&self.rule, rule))
    }
    pub fn parent_stylesheet(&self) -> Option<NativeSheet> {
        if !self.member() {
            return None;
        }
        let guard = self.owner.sheet.shared_lock.read();
        for ancestor in &self.ancestors[..self.ancestor_len] {
            if !ancestor
                .list
                .read_with(&guard)
                .0
                .iter()
                .any(|rule| Self::same_native_rule(&ancestor.rule, rule))
            {
                return None;
            }
        }
        Some(self.owner.clone())
    }
    pub fn parent_rule(&self) -> Option<NativeRule> {
        if !self.member() || self.ancestor_len == 0 {
            return None;
        }
        let ancestor = &self.ancestors[self.ancestor_len - 1];
        Some(NativeRule {
            rule: ancestor.rule.clone(),
            owner: self.owner.clone(),
            containing: ancestor.containing,
            relative: ancestor.relative,
            list: ancestor.list.clone(),
            ancestors: self.ancestors.clone(),
            ancestor_len: self.ancestor_len - 1,
            capture_charge: self.capture_charge.clone(),
        })
    }
    fn same_native_rule(first: &CssRule, second: &CssRule) -> bool {
        std::mem::discriminant(first) == std::mem::discriminant(second)
            && Self::native_identity(first) == Self::native_identity(second)
    }
    pub fn owner(&self) -> NativeSheet {
        self.owner.clone()
    }
    pub fn view(&self) -> Result<RuleView, String> {
        let guard = self.owner.sheet.shared_lock.read();
        rule_view(&self.rule, &guard)
    }
    pub fn same_rule(&self, other: &Self) -> bool {
        // CssRule discriminant plus original Arc allocation identity.
        std::mem::discriminant(&self.rule) == std::mem::discriminant(&other.rule)
            && Self::native_identity(&self.rule) == Self::native_identity(&other.rule)
    }
    fn native_identity(rule: &CssRule) -> usize {
        macro_rules! ptr {
            ($arc:expr) => {
                (&**$arc as *const _) as usize
            };
        }
        match rule {
            CssRule::Style(a) => ptr!(a),
            CssRule::Namespace(a) => ptr!(a),
            CssRule::Import(a) => ptr!(a),
            CssRule::Media(a) => ptr!(a),
            CssRule::CustomMedia(a) => ptr!(a),
            CssRule::Container(a) => ptr!(a),
            CssRule::FontFace(a) => ptr!(a),
            CssRule::FontFeatureValues(a) => ptr!(a),
            CssRule::FontPaletteValues(a) => ptr!(a),
            CssRule::CounterStyle(a) => ptr!(a),
            CssRule::Keyframes(a) => ptr!(a),
            CssRule::Margin(a) => ptr!(a),
            CssRule::Supports(a) => ptr!(a),
            CssRule::Page(a) => ptr!(a),
            CssRule::Property(a) => ptr!(a),
            CssRule::Document(a) => ptr!(a),
            CssRule::LayerBlock(a) => ptr!(a),
            CssRule::LayerStatement(a) => ptr!(a),
            CssRule::Scope(a) => ptr!(a),
            CssRule::StartingStyle(a) => ptr!(a),
            CssRule::AppearanceBase(a) => ptr!(a),
            CssRule::PositionTry(a) => ptr!(a),
            CssRule::NestedDeclarations(a) => ptr!(a),
            CssRule::ViewTransition(a) => ptr!(a),
        }
    }
}
impl NativeSheet {
    pub fn disabled(&self) -> bool {
        self.sheet
            .disabled
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn media_count(&self) -> usize {
        self.sheet
            .media
            .read_with(&self.sheet.shared_lock.read())
            .media_queries
            .len()
    }
    pub fn media_at(&self, index: usize) -> Option<String> {
        use style_traits::ToCss;
        self.sheet
            .media
            .read_with(&self.sheet.shared_lock.read())
            .media_queries
            .get(index)
            .map(|query| query.to_css_string())
    }
    pub fn media_text(&self) -> String {
        use style_traits::ToCss;
        self.sheet
            .media
            .read_with(&self.sheet.shared_lock.read())
            .to_css_string()
    }
    pub fn base_url(&self) -> String {
        let guard = self.sheet.shared_lock.read();
        self.sheet.contents(&guard).url_data.as_str().to_string()
    }
    pub fn stylesheet(&self) -> &Arc<Stylesheet> {
        &self.sheet
    }
    pub fn same_sheet(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sheet, &other.sheet)
    }
    pub fn shared_lock(&self) -> SharedRwLock {
        self.sheet.shared_lock.clone()
    }
}
#[derive(Clone)]
pub struct NativeImport {
    pub id: OccurrenceId,
    rule: Arc<Locked<ImportRule>>,
    pub parent: NativeSheet,
    pub child: Option<NativeSheet>,
    media: Arc<Locked<MediaList>>,
    pub state: ImportState,
    /// Original parsed rule-list index, not a URL or serialized-text key.
    pub index: usize,
    pub depth: usize,
}
impl NativeImport {
    /// Serialize the original retained import rule, including authored URL
    /// spelling and conditions, even after removal from its owner's list.
    pub fn view(&self) -> Result<RuleView, String> {
        let lock = self.parent.shared_lock();
        let guard = lock.read();
        let mut view=rule_view(&CssRule::Import(self.rule.clone()), &guard)?;
        use style_traits::ToCss;
        // Pending/refused imports have no child sheet, but retain their own
        // original parsed media allocation independently of loading state.
        let media=self.media.read_with(&guard);
        let text=media.to_css_string();
        view.properties.insert("media".into(),text.clone());
        if self.rule.read_with(&guard).stylesheet.as_sheet().is_none() && !media.is_empty() {
            let prefix=view.css_text.strip_suffix(';').ok_or("invalid retained import serialization")?;
            view.css_text=format!("{prefix} {text};");
        }
        Ok(view)
    }

    pub fn media_count(&self) -> usize {
        self.media
            .read_with(&self.parent.shared_lock().read())
            .media_queries
            .len()
    }
    pub fn media_at(&self, index: usize) -> Option<String> {
        use style_traits::ToCss;
        self.media
            .read_with(&self.parent.shared_lock().read())
            .media_queries
            .get(index)
            .map(|query| query.to_css_string())
    }
    /// Provider interns retained handles by same_rule/same_sheet, never by the
    /// per-graph OccurrenceId alone. Arc identity remains alive in the handle.
    pub fn layer_name(&self) -> Option<String> {
        use style_traits::ToCss;
        match &self.rule.read_with(&self.parent.shared_lock().read()).layer {
            ImportLayer::None => None,
            ImportLayer::Anonymous => Some(String::new()),
            ImportLayer::Named(name) => Some(name.to_css_string()),
        }
    }
    pub fn supports_text(&self) -> Option<String> {
        use style_traits::ToCss;
        self.rule
            .read_with(&self.parent.shared_lock().read())
            .supports
            .as_ref()
            .map(|condition| condition.condition.to_css_string())
    }
    pub fn media_text(&self) -> String {
        use style_traits::ToCss;
        self.media
            .read_with(&self.parent.shared_lock().read())
            .to_css_string()
    }
    pub fn media_queries(&self) -> Vec<String> {
        use style_traits::ToCss;
        self.media
            .read_with(&self.parent.shared_lock().read())
            .media_queries
            .iter()
            .map(|query| query.to_css_string())
            .collect()
    }
    pub fn href(&self) -> Option<String> {
        self.rule
            .read_with(&self.parent.shared_lock().read())
            .url
            .url()
            .map(|url| url.to_string())
    }
    pub fn same_rule(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.rule, &other.rule)
    }
}
/// Issued only for an attached Pending occurrence. Native transport retains
/// this together with its document/generation/cancellation and response proof.
/// No authored value can construct or substitute the private identity fields.
pub struct CompletionToken {
    id: OccurrenceId,
    parent: NativeSheet,
    rule: Arc<Locked<ImportRule>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Charge {
        live: LeaseArc<AtomicUsize>,
        bytes: usize,
    }
    impl Drop for Charge {
        fn drop(&mut self) {
            self.live.fetch_sub(self.bytes, Ordering::SeqCst);
        }
    }
    #[derive(Default)]
    struct Budget {
        live: LeaseArc<AtomicUsize>,
        refused: AtomicBool,
        calls: AtomicUsize,
        fail_at: AtomicUsize,
    }
    impl ImportAdmission for Budget {
        fn checkpoint(&self) -> Result<(), String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.refused.load(Ordering::SeqCst) || self.fail_at.load(Ordering::SeqCst) == call {
                Err("retired".into())
            } else {
                Ok(())
            }
        }
        fn reserve(&self, bytes: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            self.checkpoint()?;
            self.live.fetch_add(bytes, Ordering::SeqCst);
            Ok(LeaseArc::new(Charge {
                live: self.live.clone(),
                bytes,
            }))
        }
    }
    fn graph(source: &str, budget: &Budget) -> ImportGraph {
        ImportGraph::parse(
            source,
            "https://example.test/root.css",
            false,
            8,
            SharedRwLock::new(),
            budget,
        )
        .unwrap()
    }
    #[test]
    fn pending_refused_order_media_and_native_identity() {
        let budget = Budget::default();
        let mut graph = graph(
            "@import 'a.css' screen; @import 'a.css'; @import 'never.css' supports(unknown-property: bad);",
            &budget,
        );
        assert_eq!(graph.imports().count(), 3);
        let first = graph.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(first.index, 0);
        assert_eq!(first.state, ImportState::Pending);
        assert!(first.child.is_none());
        assert_eq!(
            graph.occurrence(OccurrenceId(3)).unwrap().state,
            ImportState::Refused
        );
        let views = graph.views(&graph.root()).unwrap();
        assert_eq!(views[0].properties.get("media").unwrap(), "screen");
        assert!(views[0].css_text.ends_with(" screen;"));
        graph
            .complete(
                first.id,
                "div { color: red }",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        graph
            .complete(
                OccurrenceId(2),
                "div { color: red }",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let completed = graph.occurrence(first.id).unwrap();
        assert!(Arc::ptr_eq(&first.rule, &completed.rule));
        assert!(Arc::ptr_eq(&first.parent.sheet, &completed.parent.sheet));
        assert!(!Arc::ptr_eq(
            &completed.child.unwrap().sheet,
            &graph
                .occurrence(OccurrenceId(2))
                .unwrap()
                .child
                .unwrap()
                .sheet
        ));
        assert!(
            !graph
                .complete(
                    OccurrenceId(3),
                    "",
                    "https://example.test/never.css",
                    &budget
                )
                .unwrap()
        );
    }
    #[test]
    fn failed_completion_is_transactional_and_detachment_retires_descendants() {
        let budget = Budget::default();
        let mut graph = graph("@import 'a.css';", &budget);
        let retained = graph.occurrence(OccurrenceId(1)).unwrap();
        let before = budget.live.load(Ordering::SeqCst);
        assert!(
            graph
                .complete(retained.id, "@import 'b.css';", "not a URL", &budget)
                .is_err()
        );
        assert_eq!(graph.imports().count(), 1);
        assert_eq!(budget.live.load(Ordering::SeqCst), before);
        assert_eq!(
            graph.occurrence(retained.id).unwrap().state,
            ImportState::Pending
        );
        graph
            .complete(
                retained.id,
                "@import 'b.css';",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let child = graph.occurrence(retained.id).unwrap().child.unwrap();
        graph.detach(retained.id).unwrap();
        assert_eq!(
            graph.occurrence(OccurrenceId(2)).unwrap().state,
            ImportState::Detached
        );
        assert!(
            !graph
                .complete(OccurrenceId(2), "", "https://example.test/b.css", &budget)
                .unwrap()
        );
        assert_eq!(graph.views(&child).unwrap().len(), 1);
        drop(graph);
        assert!(budget.live.load(Ordering::SeqCst) > 0);
        drop(child);
        drop(retained);
        assert_eq!(budget.live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn foreign_and_detached_completion_tokens_cannot_publish() {
        let budget = Budget::default();
        let mut first = graph("@import 'a.css';", &budget);
        let mut other = graph("@import 'a.css';", &budget);
        let foreign = first.begin_import(OccurrenceId(1)).unwrap();
        assert!(
            other
                .commit(foreign, "", "https://example.test/a.css", &budget)
                .is_err()
        );
        first
            .complete(
                OccurrenceId(1),
                "@import 'b.css';",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let child = first.begin_import(OccurrenceId(2)).unwrap();
        first.detach(OccurrenceId(1)).unwrap();
        assert!(
            !first
                .commit(child, "", "https://example.test/b.css", &budget)
                .unwrap()
        );
        assert!(first.occurrence(OccurrenceId(2)).unwrap().child.is_none());
    }
    #[test]
    fn native_mutation_preserves_import_arcs_and_detached_child_edits() {
        let budget = Budget::default();
        let mut graph = graph("@layer first; @import 'a.css'; p {}", &budget);
        let root = graph.root();
        let original = graph.occurrence(OccurrenceId(1)).unwrap();
        graph
            .complete(original.id, "b {}", "https://example.test/a.css", &budget)
            .unwrap();
        let child = graph.occurrence(original.id).unwrap().child.unwrap();
        let inserted = graph
            .insert_rule(&root, 1, "@import 'new.css' layer(second);", true, &budget)
            .unwrap()
            .unwrap();
        assert_eq!(graph.occurrence(original.id).unwrap().index, 2);
        assert!(original.same_rule(&graph.occurrence(original.id).unwrap()));
        assert_eq!(
            graph.insert_rule(&root, 0, "div {}", true, &budget),
            Err(RuleMutationError::HierarchyRequest)
        );
        assert_eq!(graph.occurrence(original.id).unwrap().index, 2);
        graph.delete_rule(&root, 1).unwrap();
        assert_eq!(
            graph.occurrence(inserted).unwrap().state,
            ImportState::Detached
        );
        assert_eq!(graph.occurrence(original.id).unwrap().index, 1);
        graph.delete_rule(&root, 1).unwrap();
        assert!(graph.owner(&child).is_none());
        graph
            .insert_rule(&child, 1, "i { color: blue }", true, &budget)
            .unwrap();
        graph.delete_rule(&child, 0).unwrap();
        assert_eq!(
            graph.views(&child).unwrap()[0].selector_text.as_deref(),
            Some("i")
        );
        assert!(original.same_rule(&graph.occurrence(original.id).unwrap()));
    }
    #[test]
    fn retained_group_mutations_preserve_loaded_import_and_hierarchy() {
        let budget = Budget::default();
        let mut graph = graph(
            "@import 'a.css'; @supports(display:block) { @layer theme { p {} } }",
            &budget,
        );
        graph
            .complete(
                OccurrenceId(1),
                "b {}",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let original = graph.occurrence(OccurrenceId(1)).unwrap();
        let root = graph.root();
        graph
            .insert_group_rule(&root, &[1, 0], 1, "i {}", &budget)
            .unwrap();
        graph
            .insert_group_rule(&root, &[1], 1, "section {}", &budget)
            .unwrap();
        let after = graph.views(&root).unwrap();
        assert_eq!(after[1].children.len(), 2);
        assert_eq!(after[1].children[0].children.len(), 2);
        assert_eq!(
            graph.insert_group_rule(&root, &[1, 0], 0, "@import 'bad.css';", &budget),
            Err(RuleMutationError::HierarchyRequest)
        );
        graph.delete_group_rule(&root, &[1, 0], 0).unwrap();
        let current = graph.occurrence(original.id).unwrap();
        assert!(original.same_rule(&current));
        assert!(original.child.unwrap().same_sheet(&current.child.unwrap()));
        assert_eq!(current.index, 0);
        assert_eq!(
            graph.views(&root).unwrap()[1].children[0].children[0]
                .selector_text
                .as_deref(),
            Some("i")
        );
    }
    #[test]
    fn index_preflight_failure_preserves_native_list_and_occurrences() {
        let budget = Budget::default();
        let mut graph = graph("@import 'a.css'; p {}", &budget);
        let root = graph.root();
        graph.occurrences.get_mut(&OccurrenceId(1)).unwrap().index = usize::MAX - 1;
        let before = graph.views(&root).unwrap();
        assert!(graph.insert_rule(&root, 1, "b {}", true, &budget).is_err());
        assert_eq!(graph.views(&root).unwrap(), before);
        assert_eq!(graph.imports().count(), 1);
        assert_eq!(
            graph.occurrence(OccurrenceId(1)).unwrap().index,
            usize::MAX - 1
        );
    }
    #[test]
    fn every_completion_admission_failure_preserves_pending_parent() {
        let budget = Budget::default();
        let mut probe = graph("@import 'a.css';", &budget);
        budget.calls.store(0, Ordering::SeqCst);
        probe
            .complete(
                OccurrenceId(1),
                "@import 'b.css'; p {}",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let checks = budget.calls.load(Ordering::SeqCst);
        assert!(checks > 2);
        for fail in 1..=checks {
            let budget = Budget::default();
            let mut graph = graph("@import 'a.css';", &budget);
            let root = graph.root();
            let before = graph.views(&root).unwrap();
            let charge = budget.live.load(Ordering::SeqCst);
            budget.calls.store(0, Ordering::SeqCst);
            budget.fail_at.store(fail, Ordering::SeqCst);
            assert!(
                graph
                    .complete(
                        OccurrenceId(1),
                        "@import 'b.css'; p {}",
                        "https://example.test/a.css",
                        &budget
                    )
                    .is_err(),
                "failure step {fail}"
            );
            assert_eq!(graph.views(&root).unwrap(), before);
            assert_eq!(graph.imports().count(), 1);
            let pending = graph.occurrence(OccurrenceId(1)).unwrap();
            assert_eq!(pending.state, ImportState::Pending);
            assert!(pending.child.is_none());
            assert_eq!(budget.live.load(Ordering::SeqCst), charge);
        }
    }
    #[test]
    fn native_import_properties_media_and_cross_graph_identity() {
        let budget = Budget::default();
        let mut graph = graph(
            "@import 'a.css' layer(theme) supports(display: block) screen;",
            &budget,
        );
        let import = graph.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(import.layer_name().as_deref(), Some("theme"));
        assert!(import.supports_text().unwrap().contains("display"));
        assert_eq!(import.media_text(), "screen");
        graph
            .edit_import_media(
                &import,
                super::super::MediaListEdit::Replace("print"),
                &budget,
            )
            .unwrap();
        assert_eq!(import.media_text(), "print");
        graph
            .complete(import.id, "p {}", "https://example.test/a.css", &budget)
            .unwrap();
        graph
            .edit_import_media(
                &import,
                super::super::MediaListEdit::Append("screen"),
                &budget,
            )
            .unwrap();
        assert_eq!(import.media_queries(), vec!["print", "screen"]);
        let child = graph.occurrence(import.id).unwrap().child.unwrap();
        use style_traits::ToCss;
        assert_eq!(
            child
                .sheet
                .media
                .read_with(&graph.lock.read())
                .to_css_string(),
            "print, screen"
        );
        let mut foreign = super::ImportGraph::parse(
            "@import 'a.css';",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &budget,
        )
        .unwrap();
        let foreign_import = foreign.occurrence(import.id).unwrap();
        assert!(!import.same_rule(&foreign_import));
        assert!(
            foreign
                .edit_import_media(
                    &import,
                    super::super::MediaListEdit::Replace("all"),
                    &budget
                )
                .is_err()
        );
    }
    #[test]
    fn structural_estimate_detects_tiny_rule_amplification_and_deduplicates_handles() {
        let budget = Budget::default();
        let source = "p{}".repeat(1000);
        let graph = graph(&source, &budget);
        let estimate = graph.retained_heap_estimate().unwrap();
        assert_eq!(estimate.rules, 1000);
        assert!(estimate.structural_bytes > source.len());
        let budget = Budget::default();
        let mut imports = super::ImportGraph::parse(
            "@import 'a.css';",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &budget,
        )
        .unwrap();
        imports
            .complete(
                OccurrenceId(1),
                "p {}",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let before = imports.retained_heap_estimate().unwrap();
        let clones: Vec<_> = (0..100).map(|_| imports.root()).collect();
        let after = imports.retained_heap_estimate().unwrap();
        assert_eq!(before.structural_bytes, after.structural_bytes);
        assert_eq!(after.sheets, 2);
        assert_eq!(after.rules, 2);
        drop(clones);
    }
    #[test]
    fn retained_native_rule_declaration_remains_live_after_owner_import_delete() {
        let budget = Budget::default();
        let mut graph = graph("@import 'a.css';", &budget);
        graph
            .complete(
                OccurrenceId(1),
                "p { color: red }",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let child = graph.occurrence(OccurrenceId(1)).unwrap().child.unwrap();
        let rule = graph.rule_at(&child, &[], 0, &budget).unwrap();
        let second = graph.rule_at(&child, &[], 0, &budget).unwrap();
        assert!(rule.same_rule(&second));
        graph
            .set_rule_declaration_text(&rule, "color: blue", &budget)
            .unwrap();
        assert_eq!(
            second.view().unwrap().declaration_text.as_deref(),
            Some("color: blue;")
        );
        let root = graph.root();
        graph.delete_rule(&root, 0).unwrap();
        graph
            .set_rule_declaration_text(&rule, "color: green", &budget)
            .unwrap();
        assert_eq!(
            graph.views(&child).unwrap()[0].declaration_text.as_deref(),
            Some("color: green;")
        );
        assert!(graph.owner(&child).is_none());
        drop(graph);
        assert_eq!(
            second.view().unwrap().declaration_text.as_deref(),
            Some("color: green;")
        );
    }
    #[test]
    fn property_priority_noop_and_detached_native_group_mutations() {
        let budget = Budget::default();
        let mut graph = graph("@supports(display:block) { p { color: red } }", &budget);
        let root = graph.root();
        let group = graph.rule_at(&root, &[], 0, &budget).unwrap();
        let child = graph
            .group_rules_for_handle(&group, &budget)
            .unwrap()
            .remove(0);
        graph
            .set_rule_property(&child, "color", "blue", "important", &budget)
            .unwrap();
        assert!(
            child
                .view()
                .unwrap()
                .declaration_text
                .unwrap()
                .contains("!important")
        );
        let before = child.view().unwrap();
        assert!(
            graph
                .set_rule_property(&child, "color", "green", "invalid", &budget)
                .unwrap()
                .css_text
                .is_none()
        );
        assert_eq!(child.view().unwrap(), before);
        graph
            .insert_rule(&root, 0, "aside {}", true, &budget)
            .unwrap();
        graph.delete_rule(&root, 1).unwrap();
        graph
            .insert_group_rule_handle(&group, 1, "i {}", &budget)
            .unwrap();
        assert_eq!(
            graph.group_rules_for_handle(&group, &budget).unwrap().len(),
            2
        );
        graph.delete_group_rule_handle(&group, 0).unwrap();
        assert_eq!(
            graph.group_rules_for_handle(&group, &budget).unwrap()[0]
                .view()
                .unwrap()
                .selector_text
                .as_deref(),
            Some("i")
        );
        graph
            .set_rule_property(&child, "color", "", "invalid", &budget)
            .unwrap();
        assert_eq!(child.view().unwrap().declaration_text.as_deref(), Some(""));
    }
    #[test]
    fn retained_nested_group_keeps_native_ancestor_context() {
        let budget = Budget::default();
        let mut graph = graph("article { @supports(display:block) { & > b {} } }", &budget);
        let root = graph.root();
        let group = graph.rule_at(&root, &[0], 0, &budget).unwrap();
        graph
            .insert_group_rule_handle(&group, 1, "> i { color: blue }", &budget)
            .unwrap();
        assert_eq!(
            graph.group_rules_for_handle(&group, &budget).unwrap()[1]
                .view()
                .unwrap()
                .selector_text
                .as_deref(),
            Some("& > i")
        );
        assert_eq!(
            graph.insert_group_rule_handle(&group, 0, "@import 'bad.css';", &budget),
            Err(RuleMutationError::HierarchyRequest)
        );
        graph
            .insert_rule(&root, 0, "aside {}", true, &budget)
            .unwrap();
        graph.delete_rule(&root, 1).unwrap();
        graph
            .insert_group_rule_handle(&group, 2, "> em {}", &budget)
            .unwrap();
        assert_eq!(
            graph.group_rules_for_handle(&group, &budget).unwrap()[2]
                .view()
                .unwrap()
                .selector_text
                .as_deref(),
            Some("& > em")
        );
    }
    #[test]
    fn native_rule_associations_follow_arc_membership_not_indices() {
        let budget = Budget::default();
        let mut graph = graph("p {} @supports(display:block) { b {} }", &budget);
        let root = graph.root();
        let ordinary = graph.rule_at(&root, &[], 0, &budget).unwrap();
        let group = graph.rule_at(&root, &[], 1, &budget).unwrap();
        let child = graph.group_rule_at(&group, 0, &budget).unwrap();
        assert_eq!(graph.rule_count(&root).unwrap(), 2);
        assert_eq!(graph.group_rule_count(&group).unwrap(), 1);
        assert!(child.parent_rule().unwrap().same_rule(&group));
        graph.insert_rule(&root, 0, "i {}", true, &budget).unwrap();
        assert!(ordinary.parent_stylesheet().unwrap().same_sheet(&root));
        graph.delete_rule(&root, 1).unwrap();
        assert!(ordinary.parent_stylesheet().is_none());
        graph.insert_rule(&root, 1, "p {}", true, &budget).unwrap();
        assert!(ordinary.parent_stylesheet().is_none());
        graph.delete_rule(&root, 2).unwrap();
        assert!(group.parent_stylesheet().is_none());
        assert!(child.parent_stylesheet().is_none());
        assert!(child.parent_rule().unwrap().same_rule(&group));
        graph.delete_group_rule_handle(&group, 0).unwrap();
        assert!(child.parent_rule().is_none());
    }
    #[test]
    fn generic_import_rule_lookup_preserves_occurrence_after_deletion() {
        let budget = Budget::default();
        let mut graph = graph("@import 'a.css'; @import 'a.css'; p {}", &budget);
        let root = graph.root();
        let first = graph.rule_at(&root, &[], 0, &budget).unwrap();
        let second = graph.rule_at(&root, &[], 1, &budget).unwrap();
        assert_eq!(graph.import_for_rule(&first).unwrap().id, OccurrenceId(1));
        assert_eq!(graph.import_for_rule(&second).unwrap().id, OccurrenceId(2));
        graph.delete_rule(&root, 0).unwrap();
        assert_eq!(
            graph.import_for_rule(&first).unwrap().state,
            ImportState::Detached
        );
        assert_eq!(graph.import_for_rule(&second).unwrap().id, OccurrenceId(2));
        let ordinary = graph.rule_at(&root, &[], 1, &budget).unwrap();
        assert!(graph.import_for_rule(&ordinary).is_none());
    }
    #[test]
    fn flat_ancestor_capture_admission_and_prefix_parent_are_nonrecursive() {
        let budget = Budget::default();
        let graph = graph("@supports(display:block) { @layer a { p {} } }", &budget);
        let root = graph.root();
        let child = graph.rule_at(&root, &[0, 0], 0, &budget).unwrap();
        assert_eq!(child.ancestor_len, 2);
        let parent = child.parent_rule().unwrap();
        assert_eq!(parent.ancestor_len, 1);
        assert!(LeaseArc::ptr_eq(&child.ancestors, &parent.ancestors));
        assert_eq!(parent.parent_rule().unwrap().ancestor_len, 0);
        budget.refused.store(true, Ordering::SeqCst);
        assert!(graph.rule_at(&root, &[0, 0], 0, &budget).is_err());
        drop(parent);
        drop(child);
    }
    #[test]
    fn native_media_count_item_are_live_and_missing_is_none() {
        let budget = Budget::default();
        let mut graph = graph("@import 'a.css' screen;", &budget);
        let import = graph.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(import.media_count(), 1);
        assert_eq!(import.media_at(0).as_deref(), Some("screen"));
        assert!(import.media_at(1).is_none());
        graph
            .edit_import_media(
                &import,
                super::super::MediaListEdit::Append("print"),
                &budget,
            )
            .unwrap();
        assert_eq!(import.media_count(), 2);
        assert_eq!(import.media_at(1).as_deref(), Some("print"));
        graph
            .edit_import_media(
                &import,
                super::super::MediaListEdit::Delete("screen"),
                &budget,
            )
            .unwrap();
        assert_eq!(import.media_count(), 1);
        assert_eq!(import.media_at(0).as_deref(), Some("print"));
        assert!(import.media_at(1).is_none());
    }

    #[test]
    fn every_insert_admission_failure_rolls_back_charge_and_membership() {
        let budget = Budget::default();
        let mut probe = graph("p {}", &budget);
        let root = probe.root();
        budget.calls.store(0, Ordering::SeqCst);
        probe
            .insert_rule(&root, 0, "@import 'a.css';", true, &budget)
            .unwrap();
        let checks = budget.calls.load(Ordering::SeqCst);
        for fail in 1..=checks {
            let budget = Budget::default();
            let mut graph = graph("p {}", &budget);
            let root = graph.root();
            let before = graph.views(&root).unwrap();
            let charge = budget.live.load(Ordering::SeqCst);
            budget.calls.store(0, Ordering::SeqCst);
            budget.fail_at.store(fail, Ordering::SeqCst);
            assert!(
                graph
                    .insert_rule(&root, 0, "@import 'a.css';", true, &budget)
                    .is_err(),
                "failure step {fail}"
            );
            assert_eq!(graph.views(&root).unwrap(), before);
            assert_eq!(graph.imports().count(), 0);
            assert_eq!(budget.live.load(Ordering::SeqCst), charge);
        }
    }
    #[test]
    fn native_sheet_flags_media_and_layer_names_keep_original_identity() {
        let budget = Budget::default();
        let mut graph = graph("@layer first, second.inner; @import 'a.css';", &budget);
        let root = graph.root();
        let statement = graph.rule_at(&root, &[], 0, &budget).unwrap();
        assert_eq!(statement.layer_name_count(), Some(2));
        assert_eq!(statement.layer_name_at(1).as_deref(), Some("second.inner"));
        assert!(statement.layer_name_at(2).is_none());
        assert!(!root.disabled());
        assert!(graph.set_sheet_disabled(&root, true, &budget).unwrap());
        assert!(root.disabled());
        assert!(!graph.set_sheet_disabled(&root, true, &budget).unwrap());
        graph
            .complete(
                OccurrenceId(1),
                "p {}",
                "https://example.test/a.css",
                &budget,
            )
            .unwrap();
        let import = graph.occurrence(OccurrenceId(1)).unwrap();
        let child = import.child.unwrap();
        graph
            .edit_sheet_media(
                &child,
                super::super::MediaListEdit::Replace("print"),
                &budget,
            )
            .unwrap();
        assert_eq!(child.media_count(), 1);
        assert_eq!(child.media_at(0).as_deref(), Some("print"));
        assert_eq!(graph.occurrence(import.id).unwrap().media_text(), "print");
    }
    #[test]
    fn cancellation_and_depth_do_not_publish_children() {
        let budget = Budget::default();
        let mut graph = ImportGraph::parse(
            "@import 'a.css';",
            "https://example.test/",
            false,
            0,
            SharedRwLock::new(),
            &budget,
        )
        .unwrap();
        assert!(
            graph
                .complete(OccurrenceId(1), "", "https://example.test/a.css", &budget)
                .is_err()
        );
        assert!(graph.cancel(OccurrenceId(1)).unwrap());
        assert!(
            !graph
                .complete(OccurrenceId(1), "", "https://example.test/a.css", &budget)
                .unwrap()
        );
        assert!(graph.occurrence(OccurrenceId(1)).unwrap().child.is_none());
    }
}
type ParsedSheet = (
    Arc<Stylesheet>,
    Vec<ParsedImport>,
    LeaseArc<dyn Send + Sync>,
);
type NativeGroupingList = (
    Arc<Locked<style::stylesheets::CssRules>>,
    style::stylesheets::CssRuleTypes,
    Option<style::stylesheets::CssRuleType>,
);

struct ParsedImport {
    rule: Arc<Locked<ImportRule>>,
    media: Arc<Locked<MediaList>>,
    lease: LeaseArc<dyn Send + Sync>,
}
struct Loader<'a> {
    admission: &'a dyn ImportAdmission,
    imports: RefCell<Vec<ParsedImport>>,
    error: RefCell<Option<String>>,
}
impl StylesheetLoader for Loader<'_> {
    fn request_stylesheet(
        &self,
        url: style::values::CssUrl,
        location: cssparser::SourceLocation,
        lock: &SharedRwLock,
        media: Arc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer,
    ) -> Arc<Locked<ImportRule>> {
        let refused = supports
            .as_ref()
            .is_some_and(|condition| !condition.enabled);
        let rule = Arc::new(lock.wrap(ImportRule {
            url,
            stylesheet: if refused {
                ImportSheet::new_refused()
            } else {
                ImportSheet::new_pending()
            },
            supports,
            layer,
            source_location: location,
        }));
        // The parser callback cannot return errors. Latch admission failure;
        // the entire private parse is discarded before publishing anything.
        let charge =
            std::mem::size_of::<NativeImport>().checked_add(std::mem::size_of::<ParsedImport>());
        let lease = self
            .admission
            .checkpoint()
            .and_then(|()| charge.ok_or_else(|| "import allocation overflow".to_string()))
            .and_then(|bytes| self.admission.reserve(bytes));
        match lease {
            Ok(lease) if self.error.borrow().is_none() => {
                if let Err(error) = self.imports.borrow_mut().try_reserve(1) {
                    self.error.borrow_mut().get_or_insert(error.to_string());
                    return rule;
                }
                self.imports.borrow_mut().push(ParsedImport {
                    rule: rule.clone(),
                    media,
                    lease,
                })
            }
            Err(error) => {
                self.error.borrow_mut().get_or_insert(error);
            }
            _ => (),
        }
        rule
    }
}

struct Occurrences {
    entries: Vec<NativeImport>,
    charges: Leases,
}
impl Occurrences {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            charges: LeaseArc::new(Mutex::new(Vec::new())),
        }
    }
    fn len(&self) -> usize {
        self.entries.len()
    }
    fn values(&self) -> std::slice::Iter<'_, NativeImport> {
        self.entries.iter()
    }
    fn values_mut(&mut self) -> std::slice::IterMut<'_, NativeImport> {
        self.entries.iter_mut()
    }
    fn get(&self, id: &OccurrenceId) -> Option<&NativeImport> {
        let index = usize::try_from(id.0.checked_sub(1)?).ok()?;
        self.entries.get(index)
    }
    fn get_mut(&mut self, id: &OccurrenceId) -> Option<&mut NativeImport> {
        let index = usize::try_from(id.0.checked_sub(1)?).ok()?;
        self.entries.get_mut(index)
    }
    fn reserve(
        &mut self,
        additional: usize,
        admission: &dyn ImportAdmission,
    ) -> Result<(), String> {
        let required = self
            .entries
            .len()
            .checked_add(additional)
            .ok_or("occurrence capacity overflow")?;
        if required <= self.entries.capacity() {
            return admission.checkpoint();
        }
        let old = self.entries.capacity();
        let bytes = required
            .checked_mul(std::mem::size_of::<NativeImport>())
            .ok_or("occurrence byte overflow")?;
        // Hold new allocation charge while old buffer is still live during
        // realloc. After success retain only newly added capacity charge.
        let overlap = admission.reserve(bytes)?;
        let mut charges = self
            .charges
            .lock()
            .map_err(|_| "occurrence charge ledger poisoned")?;
        charges.try_reserve(1).map_err(|e| e.to_string())?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(required)
            .map_err(|e| e.to_string())?;
        let actual = replacement.capacity();
        let delta = actual
            .checked_sub(old)
            .and_then(|count| count.checked_mul(std::mem::size_of::<NativeImport>()))
            .ok_or("occurrence delta overflow")?;
        let retained = admission.reserve(delta)?;
        admission.checkpoint()?;
        replacement.append(&mut self.entries);
        self.entries = replacement;
        charges.push(retained);
        drop(overlap);
        Ok(())
    }
}
pub struct ImportGraph {
    lock: SharedRwLock,
    root: NativeSheet,
    leases: Leases,
    next: u64,
    font_revision: LeaseArc<std::sync::atomic::AtomicU64>,
    font_descriptor_leases:LeaseArc<Mutex<FontLeaseStore>>,
    quirks: QuirksMode,
    max_depth: usize,
    occurrences: Occurrences,
}
/// Structural retained footprint, not malloc/RSS and not a complete upper
/// bound. Servo exposes no complete heap walker for opaque selector/value
/// allocations. Consumers must keep the worker RSS policy authoritative.
#[derive(Debug, Default)]
pub struct RetainedHeapEstimate {
    pub structural_bytes: usize,
    pub sheets: usize,
    pub rules: usize,
    pub opaque_rule_payloads: usize,
}
impl ImportGraph {
    pub fn set_sheet_disabled(
        &mut self,
        sheet: &NativeSheet,
        disabled: bool,
        admission: &dyn ImportAdmission,
    ) -> Result<bool, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign native sheet".into());
        }
        admission.checkpoint()?;
        Ok(sheet
            .sheet
            .disabled
            .swap(disabled, std::sync::atomic::Ordering::AcqRel)
            != disabled)
    }
    pub fn edit_sheet_media(
        &mut self,
        sheet: &NativeSheet,
        edit: super::MediaListEdit<'_>,
        admission: &dyn ImportAdmission,
    ) -> Result<super::StylesheetMediaMutation, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign native sheet".into());
        }
        let text = match edit {
            super::MediaListEdit::Replace(text)
            | super::MediaListEdit::Append(text)
            | super::MediaListEdit::Delete(text) => text,
        };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let base = sheet.sheet.contents(&self.lock.read()).url_data.clone();
        let mut context = style::parser::ParserContext::new(
            Origin::Author,
            &base,
            Some(style::stylesheets::CssRuleType::Media),
            style_traits::ParsingMode::DEFAULT,
            self.quirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let mut staged = sheet.sheet.media.read_with(&self.lock.read()).clone();
        let (changed, not_found) =
            super::media_lists::edit_native_media_list(&mut staged, &mut context, edit);
        use style_traits::ToCss;
        let queries = staged
            .media_queries
            .iter()
            .map(|query| query.to_css_string())
            .collect();
        admission.checkpoint()?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "native sheet ledger poisoned")?;
        leases.try_reserve(1).map_err(|error| error.to_string())?;
        if changed {
            *sheet.sheet.media.write_with(&mut self.lock.write()) = staged;
            leases.push(lease);
        }
        Ok(super::StylesheetMediaMutation {
            queries,
            changed,
            not_found,
        })
    }
    pub fn import_for_rule(&self, retained: &NativeRule) -> Option<NativeImport> {
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return None;
        }
        let CssRule::Import(rule) = &retained.rule else {
            return None;
        };
        self.occurrences
            .values()
            .find(|entry| {
                entry.parent.same_sheet(&retained.owner) && Arc::ptr_eq(&entry.rule, rule)
            })
            .cloned()
    }
    pub fn rule_count(&self, sheet: &NativeSheet) -> Result<usize, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign graph sheet".into());
        }
        let guard = self.lock.read();
        Ok(sheet.sheet.contents(&guard).rules.read_with(&guard).0.len())
    }
    pub fn group_rule_count(&self, group: &NativeRule) -> Result<usize, RuleMutationError> {
        let list = self.handle_group_list(group)?;
        Ok(list.read_with(&self.lock.read()).0.len())
    }
    pub fn group_rule_at(
        &self,
        group: &NativeRule,
        index: usize,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeRule, RuleMutationError> {
        let list = self.handle_group_list(group)?;
        let rule = list
            .read_with(&self.lock.read())
            .0
            .get(index)
            .cloned()
            .ok_or(RuleMutationError::IndexSize)?;
        let count = group
            .ancestor_len
            .checked_add(1)
            .ok_or_else(|| RuleMutationError::InvalidContext("ancestor count overflow".into()))?;
        let charge = admission
            .reserve(
                count
                    .checked_mul(std::mem::size_of::<RuleAncestor>())
                    .ok_or_else(|| {
                        RuleMutationError::InvalidContext("ancestor allocation overflow".into())
                    })?,
            )
            .map_err(RuleMutationError::InvalidContext)?;
        let mut ancestors = Vec::new();
        ancestors
            .try_reserve_exact(count)
            .map_err(|e| RuleMutationError::InvalidContext(e.to_string()))?;
        ancestors.extend_from_slice(&group.ancestors[..group.ancestor_len]);
        ancestors.push(RuleAncestor {
            rule: group.rule.clone(),
            list: group.list.clone(),
            containing: group.containing,
            relative: group.relative,
        });
        let mut containing = group.containing;
        let kind = group.rule.rule_type();
        containing.insert(kind);
        let relative = if matches!(
            kind,
            style::stylesheets::CssRuleType::Style | style::stylesheets::CssRuleType::Scope
        ) {
            Some(kind)
        } else {
            group.relative
        };
        admission
            .checkpoint()
            .map_err(RuleMutationError::InvalidContext)?;
        Ok(NativeRule {
            rule,
            owner: group.owner.clone(),
            containing,
            relative,
            list,
            ancestors: LeaseArc::new(ancestors),
            ancestor_len: count,
            capture_charge: charge,
        })
    }
    fn declaration_block(
        &self,
        retained: &NativeRule,
    ) -> Result<Arc<Locked<style::properties::PropertyDeclarationBlock>>, String> {
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return Err("foreign native rule".into());
        }
        let guard = self.lock.read();
        match &retained.rule {
            CssRule::Style(rule) => Ok(rule.read_with(&guard).block.clone()),
            CssRule::Page(rule) => Ok(rule.read_with(&guard).block.clone()),
            CssRule::PositionTry(rule) => Ok(rule.read_with(&guard).block.clone()),
            CssRule::NestedDeclarations(rule) => Ok(rule.read_with(&guard).block.clone()),
            _ => Err("native rule has no declaration block".into()),
        }
    }
    pub fn set_rule_property(
        &mut self,
        retained: &NativeRule,
        property: &str,
        value: &str,
        priority: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<super::DeclarationMutation, String> {
        if matches!(retained.rule, CssRule::FontFace(_)) {
            return self.mutate_font_face(retained, FontFaceDescriptorEdit::Property { name: property, value, priority }, admission);
        }
        use style::properties::declaration_block::{
            Importance, SourcePropertyDeclarationUpdate, parse_one_declaration_into,
        };
        use style::properties::{PropertyId, SourcePropertyDeclaration};
        let original = self.declaration_block(retained)?;
        let mut result = super::DeclarationMutation {
            previous_value: String::new(),
            css_text: None,
        };
        let Ok(property) = PropertyId::parse_enabled_for_all_content(property) else {
            return Ok(result);
        };
        admission.checkpoint()?;
        let lease = admission.reserve(
            value
                .len()
                .checked_add(priority.len())
                .ok_or("declaration allocation overflow")?,
        )?;
        let mut staged = original.read_with(&self.lock.read()).clone();
        staged
            .property_value_to_css(&property, &mut result.previous_value)
            .map_err(|e| e.to_string())?;
        if value.is_empty() {
            let Some(first) = staged.first_declaration_to_remove(&property) else {
                return Ok(result);
            };
            staged.remove_property(&property, first);
        } else {
            let importance = if priority.is_empty() {
                Importance::Normal
            } else if priority.eq_ignore_ascii_case("important") {
                Importance::Important
            } else {
                return Ok(result);
            };
            let base = retained
                .owner
                .sheet
                .contents(&self.lock.read())
                .url_data
                .clone();
            let mut declarations = SourcePropertyDeclaration::default();
            if parse_one_declaration_into(
                &mut declarations,
                property,
                value,
                Origin::Author,
                &base,
                None,
                style_traits::ParsingMode::DEFAULT,
                self.quirks,
                match retained.rule.rule_type() {
                    style::stylesheets::CssRuleType::NestedDeclarations => style::stylesheets::CssRuleType::Style,
                    kind => kind,
                },
            )
            .is_err()
            {
                return Ok(result);
            }
            let mut updates = SourcePropertyDeclarationUpdate::default();
            if !staged.prepare_for_update(&declarations, importance, &mut updates) {
                return Ok(result);
            }
            staged.update(declarations.drain(), importance, &mut updates);
        }
        let mut serialized = String::new();
        staged.to_css(&mut serialized).map_err(|e| e.to_string())?;
        admission.checkpoint()?;
        let mut leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        leases.try_reserve(1).map_err(|e| e.to_string())?;
        *original.write_with(&mut self.lock.write()) = staged;
        leases.push(lease);
        result.css_text = Some(serialized);
        Ok(result)
    }
    fn handle_group_list(
        &self,
        retained: &NativeRule,
    ) -> Result<Arc<Locked<style::stylesheets::CssRules>>, RuleMutationError> {
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return Err(RuleMutationError::InvalidContext(
                "foreign native group".into(),
            ));
        }
        let guard = self.lock.read();
        match &retained.rule {
            CssRule::Style(rule) => rule
                .read_with(&guard)
                .rules
                .clone()
                .ok_or(RuleMutationError::HierarchyRequest),
            CssRule::Page(rule) => Ok(rule.read_with(&guard).rules.clone()),
            CssRule::Media(rule) => Ok(rule.rules.clone()),
            CssRule::Supports(rule) => Ok(rule.rules.clone()),
            CssRule::Container(rule) => Ok(rule.rules.clone()),
            CssRule::Document(rule) => Ok(rule.rules.clone()),
            CssRule::LayerBlock(rule) => Ok(rule.rules.clone()),
            CssRule::Scope(rule) => Ok(rule.rules.clone()),
            CssRule::StartingStyle(rule) => Ok(rule.rules.clone()),
            CssRule::AppearanceBase(rule) => Ok(rule.rules.clone()),
            _ => Err(RuleMutationError::HierarchyRequest),
        }
    }
    pub fn group_rules_for_handle(
        &self,
        retained: &NativeRule,
        admission: &dyn ImportAdmission,
    ) -> Result<Vec<NativeRule>, RuleMutationError> {
        let count = self.group_rule_count(retained)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|e| RuleMutationError::InvalidContext(e.to_string()))?;
        for index in 0..count {
            output.push(self.group_rule_at(retained, index, admission)?);
        }
        Ok(output)
    }
    pub fn delete_group_rule_handle(
        &mut self,
        retained: &NativeRule,
        index: usize,
    ) -> Result<(), RuleMutationError> {
        let list = self.handle_group_list(retained)?;
        list.write_with(&mut self.lock.write()).remove_rule(index)?;
        Ok(())
    }
    pub fn insert_group_rule_handle(
        &mut self,
        retained: &NativeRule,
        index: usize,
        text: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<(), RuleMutationError> {
        let list = self.handle_group_list(retained)?;
        let context = |error: String| RuleMutationError::InvalidContext(error);
        admission.checkpoint().map_err(context)?;
        let lease = admission.reserve(text.len()).map_err(context)?;
        let mut containing = retained.containing;
        let kind = retained.rule.rule_type();
        containing.insert(kind);
        let relative = if matches!(
            kind,
            style::stylesheets::CssRuleType::Style | style::stylesheets::CssRuleType::Scope
        ) {
            Some(kind)
        } else {
            retained.relative
        };
        let loader = Loader {
            admission,
            imports: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        };
        let rule = {
            let guard = self.lock.read();
            list.read_with(&guard).parse_rule_for_insert(
                &self.lock,
                text,
                retained.owner.sheet.contents(&guard),
                index,
                containing,
                relative,
                Some(&loader),
                AllowImportRules::Yes,
            )?
        };
        if let Some(error) = loader.error.into_inner() {
            return Err(context(error));
        }
        if !loader.imports.into_inner().is_empty() {
            return Err(RuleMutationError::HierarchyRequest);
        }
        admission.checkpoint().map_err(context)?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| context("import ledger poisoned".into()))?;
        leases.try_reserve(1).map_err(|e| context(e.to_string()))?;
        list.write_with(&mut self.lock.write())
            .0
            .try_reserve(1)
            .map_err(|e| context(e.to_string()))?;
        list.write_with(&mut self.lock.write())
            .0
            .insert(index, rule);
        leases.push(lease);
        Ok(())
    }
    pub fn rule_at(
        &self,
        sheet: &NativeSheet,
        path: &[usize],
        index: usize,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeRule, RuleMutationError> {
        let (list, containing, relative) = self.group_list(sheet, path)?;
        let guard = self.lock.read();
        let rule = list
            .read_with(&guard)
            .0
            .get(index)
            .cloned()
            .ok_or(RuleMutationError::IndexSize)?;
        let charge = admission
            .reserve(
                path.len()
                    .checked_mul(std::mem::size_of::<RuleAncestor>())
                    .ok_or_else(|| {
                        RuleMutationError::InvalidContext("ancestor allocation overflow".into())
                    })?,
            )
            .map_err(RuleMutationError::InvalidContext)?;
        let mut ancestors = Vec::new();
        ancestors
            .try_reserve_exact(path.len())
            .map_err(|e| RuleMutationError::InvalidContext(e.to_string()))?;
        let mut current = sheet.sheet.contents(&guard).rules.clone();
        let mut parent_containing = style::stylesheets::CssRuleTypes::default();
        let mut parent_relative = None;
        for &position in path {
            let parent_rule = current
                .read_with(&guard)
                .0
                .get(position)
                .cloned()
                .ok_or(RuleMutationError::IndexSize)?;
            ancestors.push(RuleAncestor {
                rule: parent_rule.clone(),
                list: current.clone(),
                containing: parent_containing,
                relative: parent_relative,
            });
            let kind = parent_rule.rule_type();
            parent_containing.insert(kind);
            if matches!(
                kind,
                style::stylesheets::CssRuleType::Style | style::stylesheets::CssRuleType::Scope
            ) {
                parent_relative = Some(kind);
            }
            current = match parent_rule {
                CssRule::Style(rule) => rule
                    .read_with(&guard)
                    .rules
                    .clone()
                    .ok_or(RuleMutationError::HierarchyRequest)?,
                CssRule::Page(rule) => rule.read_with(&guard).rules.clone(),
                CssRule::Media(rule) => rule.rules.clone(),
                CssRule::Supports(rule) => rule.rules.clone(),
                CssRule::Container(rule) => rule.rules.clone(),
                CssRule::Document(rule) => rule.rules.clone(),
                CssRule::LayerBlock(rule) => rule.rules.clone(),
                CssRule::Scope(rule) => rule.rules.clone(),
                CssRule::StartingStyle(rule) => rule.rules.clone(),
                CssRule::AppearanceBase(rule) => rule.rules.clone(),
                _ => return Err(RuleMutationError::HierarchyRequest),
            };
        }
        admission
            .checkpoint()
            .map_err(RuleMutationError::InvalidContext)?;
        Ok(NativeRule {
            rule,
            owner: sheet.clone(),
            containing,
            relative,
            list,
            ancestors: LeaseArc::new(ancestors),
            ancestor_len: path.len(),
            capture_charge: charge,
        })
    }
    pub fn set_rule_declaration_text(
        &mut self,
        retained: &NativeRule,
        text: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<(), String> {
        if matches!(retained.rule, CssRule::FontFace(_)) {
            self.mutate_font_face(retained, FontFaceDescriptorEdit::Text(text), admission)?;
            return Ok(());
        }
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return Err("foreign native rule".into());
        }
        let (block, kind) = {
            let guard = self.lock.read();
            let block = match &retained.rule {
                CssRule::Style(rule) => rule.read_with(&guard).block.clone(),
                CssRule::Page(rule) => rule.read_with(&guard).block.clone(),
                CssRule::PositionTry(rule) => rule.read_with(&guard).block.clone(),
                CssRule::NestedDeclarations(rule) => rule.read_with(&guard).block.clone(),
                _ => return Err("native rule has no declaration block".into()),
            };
            // Nested declarations use normal style-property grammar, not the
            // internal CSSOM rule tag (which is preserved on the rule itself).
            let kind = match retained.rule.rule_type() {
                style::stylesheets::CssRuleType::NestedDeclarations => style::stylesheets::CssRuleType::Style,
                kind => kind,
            };
            (block, kind)
        };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let base = retained
            .owner
            .sheet
            .contents(&self.lock.read())
            .url_data
            .clone();
        let staged = style::properties::parse_style_attribute(text, &base, None, self.quirks, kind);
        admission.checkpoint()?;
        let mut leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        leases.try_reserve(1).map_err(|error| error.to_string())?;
        *block.write_with(&mut self.lock.write()) = staged;
        leases.push(lease);
        Ok(())
    }
    pub fn retained_heap_estimate(&self) -> Result<RetainedHeapEstimate, String> {
        use style::stylesheets::CssRules;
        fn add(total: &mut usize, bytes: usize) -> Result<(), String> {
            *total = total
                .checked_add(bytes)
                .ok_or("retained heap estimate overflow")?;
            Ok(())
        }
        fn capacity<T>(total: &mut usize, capacity: usize) -> Result<(), String> {
            add(
                total,
                capacity
                    .checked_mul(std::mem::size_of::<T>())
                    .ok_or("retained capacity overflow")?,
            )
        }
        fn identity<T>(arc: &Arc<T>) -> usize {
            (&**arc as *const T) as usize
        }
        let guard = self.lock.read();
        let mut result = RetainedHeapEstimate::default();
        let mut seen = std::collections::BTreeSet::new();
        let mut sheets = Vec::new();
        let mut lists: Vec<Arc<Locked<CssRules>>> = Vec::new();
        sheets
            .try_reserve(
                self.occurrences
                    .len()
                    .checked_mul(2)
                    .and_then(|count| count.checked_add(1))
                    .ok_or("sheet count overflow")?,
            )
            .map_err(|error| error.to_string())?;
        sheets.push(self.root.clone());
        for entry in self.occurrences.values() {
            sheets.push(entry.parent.clone());
            if let Some(child) = &entry.child {
                sheets.push(child.clone());
            }
        }
        add(&mut result.structural_bytes, std::mem::size_of::<Self>())?;
        capacity::<NativeImport>(
            &mut result.structural_bytes,
            self.occurrences.entries.capacity(),
        )?;
        let leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        capacity::<LeaseArc<dyn Send + Sync>>(&mut result.structural_bytes, leases.capacity())?;
        for sheet in sheets {
            if !seen.insert(identity(&sheet.sheet)) {
                continue;
            }
            result.sheets += 1;
            add(
                &mut result.structural_bytes,
                std::mem::size_of::<Stylesheet>(),
            )?;
            let contents = sheet.sheet.contents(&guard);
            add(
                &mut result.structural_bytes,
                std::mem::size_of_val(contents),
            )?;
            for text in [&contents.source_url, &contents.source_map_url]
                .into_iter()
                .flatten()
            {
                add(&mut result.structural_bytes, text.capacity())?;
            }
            lists.try_reserve(1).map_err(|error| error.to_string())?;
            lists.push(contents.rules.clone());
        }
        while let Some(list) = lists.pop() {
            if !seen.insert(identity(&list)) {
                continue;
            }
            let rules = list.read_with(&guard);
            add(
                &mut result.structural_bytes,
                std::mem::size_of::<Locked<CssRules>>(),
            )?;
            capacity::<CssRule>(&mut result.structural_bytes, rules.0.capacity())?;
            for rule in &rules.0 {
                macro_rules! measure {
                    ($arc:expr) => {{
                        let arc = $arc;
                        if !seen.insert(identity(arc)) {
                            continue;
                        }
                        add(&mut result.structural_bytes, std::mem::size_of_val(&**arc))?;
                    }};
                }
                match rule {
                    CssRule::Style(arc) => {
                        measure!(arc);
                        if let Some(list) = &arc.read_with(&guard).rules {
                            lists.try_reserve(1).map_err(|e| e.to_string())?;
                            lists.push(list.clone());
                        }
                    }
                    CssRule::Import(arc) => measure!(arc),
                    CssRule::Namespace(arc) => measure!(arc),
                    CssRule::Media(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::Supports(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::LayerBlock(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::Container(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::Document(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::Scope(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::StartingStyle(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::AppearanceBase(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.rules.clone());
                    }
                    CssRule::Page(arc) => {
                        measure!(arc);
                        lists.try_reserve(1).map_err(|e| e.to_string())?;
                        lists.push(arc.read_with(&guard).rules.clone());
                    }
                    CssRule::FontFace(arc) => measure!(arc),
                    CssRule::FontFeatureValues(arc) => measure!(arc),
                    CssRule::FontPaletteValues(arc) => measure!(arc),
                    CssRule::CounterStyle(arc) => measure!(arc),
                    CssRule::Keyframes(arc) => measure!(arc),
                    CssRule::Margin(arc) => measure!(arc),
                    CssRule::Property(arc) => measure!(arc),
                    CssRule::LayerStatement(arc) => measure!(arc),
                    CssRule::PositionTry(arc) => measure!(arc),
                    CssRule::NestedDeclarations(arc) => measure!(arc),
                    CssRule::ViewTransition(arc) => measure!(arc),
                    CssRule::CustomMedia(arc) => measure!(arc),
                }
                result.rules += 1;
                result.opaque_rule_payloads += 1;
            }
        }
        Ok(result)
    }
    pub fn edit_import_media(
        &mut self,
        retained: &NativeImport,
        edit: super::MediaListEdit<'_>,
        admission: &dyn ImportAdmission,
    ) -> Result<super::StylesheetMediaMutation, String> {
        if !LeaseArc::ptr_eq(&retained.parent.leases, &self.leases)
            || !self
                .occurrences
                .get(&retained.id)
                .is_some_and(|entry| entry.same_rule(retained))
        {
            return Err("foreign native import media handle".into());
        }
        let text = match edit {
            super::MediaListEdit::Replace(text)
            | super::MediaListEdit::Append(text)
            | super::MediaListEdit::Delete(text) => text,
        };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let base = {
            let guard = self.lock.read();
            retained.parent.sheet.contents(&guard).url_data.clone()
        };
        let mut context = style::parser::ParserContext::new(
            Origin::Author,
            &base,
            Some(style::stylesheets::CssRuleType::Media),
            style_traits::ParsingMode::DEFAULT,
            self.quirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        // Clone only the native media list, not the stylesheet or import rule.
        let mut staged = retained.media.read_with(&self.lock.read()).clone();
        let (changed, not_found) =
            super::media_lists::edit_native_media_list(&mut staged, &mut context, edit);
        use style_traits::ToCss;
        let queries = staged
            .media_queries
            .iter()
            .map(|query| query.to_css_string())
            .collect();
        admission.checkpoint()?;
        let mut leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        leases.try_reserve(1).map_err(|error| error.to_string())?;
        if changed {
            *retained.media.write_with(&mut self.lock.write()) = staged;
            leases.push(lease);
        }
        Ok(super::StylesheetMediaMutation {
            queries,
            changed,
            not_found,
        })
    }
    /// Resolve a native grouping list from the original owning sheet. Imported
    /// sheets are separate owners; paths cannot cross an import boundary.
    fn group_list(
        &self,
        sheet: &NativeSheet,
        path: &[usize],
    ) -> Result<NativeGroupingList, RuleMutationError> {
        use style::stylesheets::{CssRuleType, CssRuleTypes};
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err(RuleMutationError::InvalidContext(
                "foreign graph sheet".into(),
            ));
        }
        let guard = self.lock.read();
        let mut list = sheet.sheet.contents(&guard).rules.clone();
        let mut containing = CssRuleTypes::default();
        let mut relative = None;
        for &index in path {
            let rule = list
                .read_with(&guard)
                .0
                .get(index)
                .cloned()
                .ok_or(RuleMutationError::IndexSize)?;
            let kind = rule.rule_type();
            containing.insert(kind);
            if matches!(kind, CssRuleType::Style | CssRuleType::Scope) {
                relative = Some(kind);
            }
            list = match rule {
                CssRule::Style(rule) => rule
                    .read_with(&guard)
                    .rules
                    .clone()
                    .ok_or(RuleMutationError::HierarchyRequest)?,
                CssRule::Page(rule) => rule.read_with(&guard).rules.clone(),
                CssRule::Media(rule) => rule.rules.clone(),
                CssRule::Supports(rule) => rule.rules.clone(),
                CssRule::Container(rule) => rule.rules.clone(),
                CssRule::Document(rule) => rule.rules.clone(),
                CssRule::LayerBlock(rule) => rule.rules.clone(),
                CssRule::Scope(rule) => rule.rules.clone(),
                CssRule::StartingStyle(rule) => rule.rules.clone(),
                CssRule::AppearanceBase(rule) => rule.rules.clone(),
                _ => return Err(RuleMutationError::HierarchyRequest),
            };
        }
        Ok((list, containing, relative))
    }
    pub fn insert_group_rule(
        &mut self,
        sheet: &NativeSheet,
        path: &[usize],
        index: usize,
        text: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<(), RuleMutationError> {
        if path.is_empty() {
            return self
                .insert_rule(sheet, index, text, true, admission)
                .map(|_| ());
        }
        let (list, containing, relative) = self.group_list(sheet, path)?;
        let context = |error: String| RuleMutationError::InvalidContext(error);
        admission.checkpoint().map_err(context)?;
        let lease = admission.reserve(text.len()).map_err(context)?;
        let loader = Loader {
            admission,
            imports: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        };
        let rule = {
            let guard = self.lock.read();
            list.read_with(&guard).parse_rule_for_insert(
                &self.lock,
                text,
                sheet.sheet.contents(&guard),
                index,
                containing,
                relative,
                Some(&loader),
                AllowImportRules::Yes,
            )?
        };
        if let Some(error) = loader.error.into_inner() {
            return Err(context(error));
        }
        // Native hierarchy rejects imports in groups; do not invent import
        // occurrences for invalid nested imports or touch unrelated indices.
        if !loader.imports.into_inner().is_empty() {
            return Err(RuleMutationError::HierarchyRequest);
        }
        admission.checkpoint().map_err(context)?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| context("import ledger poisoned".into()))?;
        leases
            .try_reserve(1)
            .map_err(|error| context(error.to_string()))?;
        list.write_with(&mut self.lock.write())
            .0
            .try_reserve(1)
            .map_err(|error| context(error.to_string()))?;
        list.write_with(&mut self.lock.write())
            .0
            .insert(index, rule);
        leases.push(lease);
        Ok(())
    }
    pub fn delete_group_rule(
        &mut self,
        sheet: &NativeSheet,
        path: &[usize],
        index: usize,
    ) -> Result<(), RuleMutationError> {
        if path.is_empty() {
            return self.delete_rule(sheet, index);
        }
        let (list, _, _) = self.group_list(sheet, path)?;
        list.write_with(&mut self.lock.write()).remove_rule(index)?;
        Ok(())
    }
    /// Live owning association; a retained detached child stays editable but
    /// no longer reports an owning parent sheet or import rule.
    pub fn owner(&self, sheet: &NativeSheet) -> Option<NativeImport> {
        self.occurrences
            .values()
            .find(|entry| {
                entry.state == ImportState::Complete
                    && entry
                        .child
                        .as_ref()
                        .is_some_and(|child| child.same_sheet(sheet))
            })
            .cloned()
    }
    pub fn insert_rule(
        &mut self,
        sheet: &NativeSheet,
        index: usize,
        text: &str,
        allow_imports: bool,
        admission: &dyn ImportAdmission,
    ) -> Result<Option<OccurrenceId>, RuleMutationError> {
        use style::stylesheets::CssRuleTypes;
        let context = |error: String| RuleMutationError::InvalidContext(error);
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err(context("foreign graph sheet".into()));
        }
        admission.checkpoint().map_err(context)?;
        let source_lease = admission.reserve(text.len()).map_err(context)?;
        let loader = Loader {
            admission,
            imports: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        };
        let list = sheet.sheet.contents(&self.lock.read()).rules.clone();
        let rule = {
            let guard = self.lock.read();
            list.read_with(&guard).parse_rule_for_insert(
                &self.lock,
                text,
                sheet.sheet.contents(&guard),
                index,
                CssRuleTypes::default(),
                None,
                Some(&loader),
                if allow_imports {
                    AllowImportRules::Yes
                } else {
                    AllowImportRules::No
                },
            )?
        };
        if let Some(error) = loader.error.into_inner() {
            return Err(context(error));
        }
        let mut imports = loader.imports.into_inner();
        let next = self
            .next
            .checked_add(
                u64::try_from(imports.len())
                    .map_err(|_| context("import identity overflow".into()))?,
            )
            .ok_or_else(|| context("import identity exhausted".into()))?;
        let depth = self
            .occurrences
            .values()
            .find(|entry| {
                entry
                    .child
                    .as_ref()
                    .is_some_and(|child| child.same_sheet(sheet))
            })
            .map_or(Some(1), |entry| entry.depth.checked_add(1))
            .ok_or_else(|| context("import depth overflow".into()))?;
        for entry in self.occurrences.values() {
            if entry.parent.same_sheet(sheet) && entry.index != usize::MAX && entry.index >= index {
                entry
                    .index
                    .checked_add(1)
                    .filter(|value| *value != usize::MAX)
                    .ok_or_else(|| context("import rule index overflow".into()))?;
            }
        }
        admission.checkpoint().map_err(context)?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| context("import ledger poisoned".into()))?;
        leases
            .try_reserve(
                imports
                    .len()
                    .checked_add(1)
                    .ok_or_else(|| context("lease count overflow".into()))?,
            )
            .map_err(|error| context(error.to_string()))?;
        list.write_with(&mut self.lock.write())
            .0
            .try_reserve(1)
            .map_err(|error| context(error.to_string()))?;
        // Stage sequential native entries, then admit owning capacity before
        // any native rule-list or occurrence publication.
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(imports.len())
            .map_err(|error| context(error.to_string()))?;
        let mut staged_charge = None;
        let id = if let Some(entry) = imports.pop() {
            let guard = self.lock.read();
            let state = if matches!(
                entry.rule.read_with(&guard).stylesheet,
                ImportSheet::Refused
            ) {
                ImportState::Refused
            } else {
                ImportState::Pending
            };
            let id = OccurrenceId(self.next);
            staged_charge = Some(entry.lease);
            staged.push(NativeImport {
                id,
                rule: entry.rule,
                parent: sheet.clone(),
                child: None,
                media: entry.media,
                state,
                index,
                depth,
            });
            Some(id)
        } else {
            None
        };
        self.occurrences
            .reserve(staged.len(), admission)
            .map_err(context)?;
        if let Some(charge) = staged_charge {
            leases.push(charge);
        }
        list.write_with(&mut self.lock.write())
            .0
            .insert(index, rule);
        for entry in self.occurrences.values_mut() {
            if Some(entry.id) != id
                && entry.parent.same_sheet(sheet)
                && entry.index != usize::MAX
                && entry.index >= index
            {
                entry.index += 1;
            }
        }
        self.occurrences.entries.append(&mut staged);
        leases.push(source_lease);
        self.next = next;
        Ok(id)
    }
    pub fn delete_rule(
        &mut self,
        sheet: &NativeSheet,
        index: usize,
    ) -> Result<(), RuleMutationError> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err(RuleMutationError::InvalidContext(
                "foreign graph sheet".into(),
            ));
        }
        let list = sheet.sheet.contents(&self.lock.read()).rules.clone();
        let id = self
            .occurrences
            .values()
            .find(|entry| entry.parent.same_sheet(sheet) && entry.index == index)
            .map(|entry| entry.id);
        // Stylo validates index/namespace restrictions before removing a rule.
        list.write_with(&mut self.lock.write()).remove_rule(index)?;
        if let Some(id) = id {
            if let Some(entry) = self.occurrences.get_mut(&id) {
                entry.state = ImportState::Detached;
                entry.index = usize::MAX;
            }
            self.propagate_detachment();
        }
        for entry in self.occurrences.values_mut() {
            if entry.parent.same_sheet(sheet) && entry.index != usize::MAX && entry.index > index {
                entry.index -= 1;
            }
        }
        Ok(())
    }
    /// max_depth is the existing host expansion limit, not a new default.
    pub fn parse(
        source: &str,
        base: &str,
        quirks: bool,
        max_depth: usize,
        lock: SharedRwLock,
        admission: &dyn ImportAdmission,
    ) -> Result<Self, String> {
        let quirks = if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        };
        let (sheet, imports, source_lease) = Self::parse_sheet(
            source,
            base,
            quirks,
            &lock,
            Arc::new(lock.wrap(MediaList::empty())),
            admission,
        )?;
        let font_epoch_charge = admission.reserve(
            std::mem::size_of::<std::sync::atomic::AtomicU64>()
                .checked_add(2 * std::mem::size_of::<usize>())
                .ok_or("font epoch allocation overflow")?,
        )?;
        let mut initial_leases = Vec::new();
        initial_leases
            .try_reserve(3)
            .map_err(|error| error.to_string())?;
        initial_leases.push(source_lease);
        initial_leases.push(font_epoch_charge);
        let leases = LeaseArc::new(Mutex::new(initial_leases));
        let root = NativeSheet {
            sheet,
            leases: leases.clone(),
        };
        let mut graph = Self {
            lock,
            root: root.clone(),
            leases: leases.clone(),
            next: 1,
            font_revision: LeaseArc::new(std::sync::atomic::AtomicU64::new(0)),
            font_descriptor_leases:LeaseArc::new(Mutex::new(FontLeaseStore::default())),
            quirks,
            max_depth,
            occurrences: Occurrences::new(),
        };
        graph
            .leases
            .lock()
            .map_err(|_| "import ledger poisoned")?
            .push(graph.occurrences.charges.clone());
        graph.commit_imports(root, imports, 1, None, admission)?;
        Ok(graph)
    }
    fn parse_sheet(
        source: &str,
        base: &str,
        quirks: QuirksMode,
        lock: &SharedRwLock,
        media: Arc<Locked<MediaList>>,
        admission: &dyn ImportAdmission,
    ) -> Result<ParsedSheet, String> {
        admission.checkpoint()?;
        let url = url::Url::parse(base).map_err(|error| error.to_string())?;
        let lease = admission.reserve(
            source
                .len()
                .checked_add(std::mem::size_of::<Stylesheet>())
                .ok_or("stylesheet allocation overflow")?,
        )?;
        let loader = Loader {
            admission,
            imports: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        };
        let sheet = Stylesheet::from_str(
            source,
            UrlExtraData::from(url),
            Origin::Author,
            media,
            lock.clone(),
            Some(&loader),
            None,
            quirks,
            AllowImportRules::Yes,
        );
        if let Some(error) = loader.error.into_inner() {
            return Err(error);
        }
        admission.checkpoint()?;
        Ok((Arc::new(sheet), loader.imports.into_inner(), lease))
    }
    fn commit_imports(
        &mut self,
        parent: NativeSheet,
        imports: Vec<ParsedImport>,
        depth: usize,
        source_lease: Option<LeaseArc<dyn Send + Sync>>,
        admission: &dyn ImportAdmission,
    ) -> Result<(), String> {
        let next = self
            .next
            .checked_add(u64::try_from(imports.len()).map_err(|_| "import identity overflow")?)
            .ok_or("import identity exhausted")?;
        let guard = self.lock.read();
        // Stylo accepts imports only at top level. Find their exact positions
        // without cloning the complete native rule vector.
        let contents = parent.sheet.contents(&guard);
        let rules = &contents.rules.read_with(&guard).0;
        let mut positions = rules
            .iter()
            .enumerate()
            .filter_map(|(index, rule)| matches!(rule, CssRule::Import(_)).then_some(index));
        let count = imports
            .len()
            .checked_add(usize::from(source_lease.is_some()))
            .ok_or("import lease count overflow")?;
        let mut staged_leases = Vec::new();
        staged_leases
            .try_reserve(count)
            .map_err(|error| error.to_string())?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(imports.len())
            .map_err(|error| error.to_string())?;
        for (offset, entry) in imports.into_iter().enumerate() {
            let index = positions
                .next()
                .ok_or("native import occurrence mismatch")?;
            let state = if matches!(
                entry.rule.read_with(&guard).stylesheet,
                ImportSheet::Refused
            ) {
                ImportState::Refused
            } else {
                ImportState::Pending
            };
            staged_leases.push(entry.lease);
            let id = OccurrenceId(self.next + offset as u64);
            staged.push(NativeImport {
                id,
                rule: entry.rule,
                parent: parent.clone(),
                child: None,
                media: entry.media,
                state,
                index,
                depth,
            });
        }
        if positions.next().is_some() {
            return Err("native import occurrence mismatch".into());
        }
        if let Some(lease) = source_lease {
            staged_leases.push(lease);
        }
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "import lease ledger poisoned")?;
        leases
            .try_reserve(count)
            .map_err(|error| error.to_string())?;
        admission.checkpoint()?;
        self.occurrences.reserve(staged.len(), admission)?;
        self.occurrences.entries.append(&mut staged);
        leases.append(&mut staged_leases);
        self.next = next;
        Ok(())
    }
    pub fn root(&self) -> NativeSheet {
        self.root.clone()
    }
    pub fn shared_lock(&self) -> SharedRwLock {
        self.lock.clone()
    }
    pub fn occurrence(&self, id: OccurrenceId) -> Option<NativeImport> {
        self.occurrences.get(&id).cloned()
    }
    pub fn imports(&self) -> impl Iterator<Item = &NativeImport> {
        self.occurrences.values()
    }
    pub fn views(&self, sheet: &NativeSheet) -> Result<Vec<RuleView>, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign import graph sheet".into());
        }
        let guard = self.lock.read();
        sheet
            .sheet
            .contents(&guard)
            .rules
            .read_with(&guard)
            .0
            .iter()
            .map(|rule| {
                let mut view = rule_view(rule, &guard)?;
                if let CssRule::Import(rule) = rule
                    && let Some(entry) = self
                        .occurrences
                        .values()
                        .find(|entry| Arc::ptr_eq(&entry.rule, rule))
                {
                    use style_traits::ToCss;
                    let media = entry.media.read_with(&guard);
                    let text = media.to_css_string();
                    view.properties.insert("media".into(), text.clone());
                    if rule.read_with(&guard).stylesheet.as_sheet().is_none() && !media.is_empty() {
                        let end = view
                            .css_text
                            .strip_suffix(';')
                            .ok_or("invalid native import serialization")?;
                        view.css_text = format!("{end} {text};");
                    }
                }
                Ok(view)
            })
            .collect()
    }
    /// Broker authorization precedes this call. No identity is keyed by URL;
    /// two occurrences loading the same URL get two distinct parsed sheets.
    fn complete(
        &mut self,
        id: OccurrenceId,
        source: &str,
        final_url: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<bool, String> {
        let entry = self
            .occurrences
            .get(&id)
            .ok_or("unknown import occurrence")?;
        if entry.state != ImportState::Pending {
            return Ok(false);
        }
        if entry.depth > self.max_depth {
            return Err("stylesheet import depth exceeded".into());
        }
        let depth = entry.depth;
        let (child, imports, lease) = Self::parse_sheet(
            source,
            final_url,
            self.quirks,
            &self.lock,
            entry.media.clone(),
            admission,
        )?;
        self.next
            .checked_add(u64::try_from(imports.len()).map_err(|_| "import identity overflow")?)
            .ok_or("import identity exhausted")?;
        admission.checkpoint()?;
        let child = NativeSheet {
            sheet: child,
            leases: self.leases.clone(),
        };
        // Index first; it performs every fallible operation before changing
        // the original ImportRule. Failed admission leaves the parent pending.
        // A poisoned ledger is detected before indexing; no other code has
        // access to its Mutex, and commit_imports does not execute callbacks.
        drop(
            self.leases
                .lock()
                .map_err(|_| "import lease ledger poisoned")?,
        );
        self.commit_imports(
            child.clone(),
            imports,
            depth.checked_add(1).ok_or("import depth overflow")?,
            Some(lease),
            admission,
        )?;
        let entry = self.occurrences.get_mut(&id).expect("validated occurrence");
        entry.rule.write_with(&mut self.lock.write()).stylesheet =
            ImportSheet::new(child.sheet.clone());
        entry.child = Some(child);
        entry.state = ImportState::Complete;
        Ok(true)
    }
    pub fn begin_import(&self, id: OccurrenceId) -> Option<CompletionToken> {
        self.occurrences
            .get(&id)
            .filter(|entry| entry.state == ImportState::Pending)
            .map(|entry| CompletionToken {
                id,
                parent: entry.parent.clone(),
                rule: entry.rule.clone(),
            })
    }
    pub fn commit(
        &mut self,
        token: CompletionToken,
        source: &str,
        final_url: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<bool, String> {
        if !LeaseArc::ptr_eq(&token.parent.leases, &self.leases) {
            return Err("foreign import completion token".into());
        }
        let entry = self
            .occurrences
            .get(&token.id)
            .ok_or("unknown import completion token")?;
        if !Arc::ptr_eq(&entry.rule, &token.rule) || !entry.parent.same_sheet(&token.parent) {
            return Err("stale import completion token".into());
        }
        self.complete(token.id, source, final_url, admission)
    }
    pub fn cancel(&mut self, id: OccurrenceId) -> Result<bool, String> {
        let entry = self
            .occurrences
            .get_mut(&id)
            .ok_or("unknown import occurrence")?;
        if entry.state != ImportState::Pending {
            return Ok(false);
        }
        entry.state = ImportState::Cancelled;
        entry.rule.write_with(&mut self.lock.write()).stylesheet = ImportSheet::new_refused();
        Ok(true)
    }
    /// Delete/replace invalidates descendants' pending completions but retains
    /// original rule and child Arc identities for independently held wrappers.
    pub fn detach(&mut self, id: OccurrenceId) -> Result<(), String> {
        self.occurrences
            .get(&id)
            .ok_or("unknown import occurrence")?;
        self.occurrences
            .get_mut(&id)
            .expect("validated occurrence")
            .state = ImportState::Detached;
        // IDs are allocated parent-before-child. One ordered sweep propagates
        // detachment without an unadmitted work queue.
        self.propagate_detachment();
        Ok(())
    }
    fn propagate_detachment(&mut self) {
        let ids = self.next;
        for raw in 1..ids {
            let key = OccurrenceId(raw);
            let detached = self.occurrences.get(&key).is_some_and(|entry| {
                self.occurrences.values().any(|ancestor| {
                    ancestor.state == ImportState::Detached
                        && ancestor
                            .child
                            .as_ref()
                            .is_some_and(|child| Arc::ptr_eq(&entry.parent.sheet, &child.sheet))
                })
            });
            if detached {
                self.occurrences
                    .get_mut(&key)
                    .expect("existing occurrence")
                    .state = ImportState::Detached;
            }
        }
    }
}

pub struct PreparedNativeSheetMedia {
    sheet: NativeSheet,
    media: MediaList,
    lease: LeaseArc<dyn Send + Sync>,
    pub result: super::StylesheetMediaMutation,
}
impl ImportGraph {
    pub fn prepare_sheet_media(
        &self,
        sheet: &NativeSheet,
        edit: super::MediaListEdit<'_>,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedNativeSheetMedia, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign native sheet".into());
        }
        let text = match edit {
            super::MediaListEdit::Replace(text)
            | super::MediaListEdit::Append(text)
            | super::MediaListEdit::Delete(text) => text,
        };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let base = sheet.sheet.contents(&self.lock.read()).url_data.clone();
        let mut context = style::parser::ParserContext::new(
            Origin::Author,
            &base,
            Some(style::stylesheets::CssRuleType::Media),
            style_traits::ParsingMode::DEFAULT,
            self.quirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let mut media = sheet.sheet.media.read_with(&self.lock.read()).clone();
        let (changed, not_found) =
            super::media_lists::edit_native_media_list(&mut media, &mut context, edit);
        use style_traits::ToCss;
        let queries = media
            .media_queries
            .iter()
            .map(|query| query.to_css_string())
            .collect();
        sheet
            .leases
            .lock()
            .map_err(|_| "native sheet ledger poisoned")?
            .try_reserve(1)
            .map_err(|error| error.to_string())?;
        admission.checkpoint()?;
        Ok(PreparedNativeSheetMedia {
            sheet: sheet.clone(),
            media,
            lease,
            result: super::StylesheetMediaMutation {
                queries,
                changed,
                not_found,
            },
        })
    }
}
impl PreparedNativeSheetMedia {
    pub fn media_text(&self) -> String {
        use style_traits::ToCss;
        self.media.to_css_string()
    }
    pub fn commit(self) -> super::StylesheetMediaMutation {
        if self.result.changed {
            let mut leases = self.sheet.leases.lock().expect("prepared native ledger");
            *self
                .sheet
                .sheet
                .media
                .write_with(&mut self.sheet.sheet.shared_lock.write()) = self.media;
            leases.push(self.lease);
        }
        self.result
    }
}

pub struct PreparedNativeSheetDisabled {
    sheet: NativeSheet,
    value: bool,
    pub changed: bool,
}
impl ImportGraph {
    pub fn prepare_sheet_disabled(
        &self,
        sheet: &NativeSheet,
        value: bool,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedNativeSheetDisabled, String> {
        if !LeaseArc::ptr_eq(&sheet.leases, &self.leases) {
            return Err("foreign native sheet".into());
        }
        admission.checkpoint()?;
        Ok(PreparedNativeSheetDisabled {
            sheet: sheet.clone(),
            value,
            changed: sheet.disabled() != value,
        })
    }
}
impl PreparedNativeSheetDisabled {
    pub fn commit(self) -> bool {
        if self.changed {
            self.sheet
                .sheet
                .disabled
                .store(self.value, std::sync::atomic::Ordering::Release);
        }
        self.changed
    }
}

pub struct NativeDeclarationView {
    pub view: super::DeclarationView,
    pub allocation: LeaseArc<dyn Send + Sync>,
}
impl ImportGraph {
    /// Read the original declaration block, never parse a serialized rule.
    /// Pinned Stylo serializes to CssStringWriter only: serializer scratch is
    /// currently infallible and not a prospective physical allocation bound.
    pub fn rule_declaration(
        &self,
        retained: &NativeRule,
        property: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeDeclarationView, String> {
        if matches!(retained.rule, CssRule::FontFace(_)) {
            return self.font_face_declaration(retained, property, admission);
        }
        use style::properties::PropertyId;
        admission.checkpoint()?;
        let original = self.declaration_block(retained)?;
        let guard = self.lock.read();
        let block = original.read_with(&guard);
        let mut result = super::DeclarationView {
            css_text: String::new(),
            names: Vec::new(),
            value: String::new(),
            important: false,
        };
        block
            .to_css(&mut result.css_text)
            .map_err(|e| e.to_string())?;
        if let Ok(property) = PropertyId::parse_enabled_for_all_content(property) {
            block
                .property_value_to_css(&property, &mut result.value)
                .map_err(|e| e.to_string())?;
            result.important = block.property_priority(&property).important();
        }
        let count = block.declaration_importance_iter().count();
        result
            .names
            .try_reserve_exact(count)
            .map_err(|e| e.to_string())?;
        for (declaration, _) in block.declaration_importance_iter() {
            let name = declaration.id().name();
            let mut owned = String::new();
            owned
                .try_reserve_exact(name.len())
                .map_err(|e| e.to_string())?;
            owned.push_str(&name);
            result.names.push(owned);
        }
        drop(guard);
        admission.checkpoint()?;
        let bytes = result
            .css_text
            .capacity()
            .checked_add(result.value.capacity())
            .and_then(|bytes| {
                bytes.checked_add(
                    result
                        .names
                        .capacity()
                        .checked_mul(std::mem::size_of::<String>())?,
                )
            })
            .and_then(|bytes| {
                result
                    .names
                    .iter()
                    .try_fold(bytes, |bytes, name| bytes.checked_add(name.capacity()))
            })
            .ok_or("declaration retained view overflow")?;
        let allocation = admission.reserve(bytes)?;
        Ok(NativeDeclarationView {
            view: result,
            allocation,
        })
    }
}
impl ImportGraph {
    /// CSSOM invalid selector is a no-op, not SyntaxError. Mutates only the
    /// retained StyleRule selector list; declaration/import/group Arcs survive.
    pub fn set_rule_selector_text(
        &mut self,
        retained: &NativeRule,
        text: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<bool, String> {
        use selectors::{SelectorList, parser::ParseRelative};
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return Err("foreign native rule".into());
        }
        if matches!(&retained.rule, CssRule::Page(_)) {
            return self.set_page_selector_text(retained,text,admission);
        }
        let CssRule::Style(rule) = &retained.rule else {
            return Err("native rule is not a style rule".into());
        };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let selectors = {
            let guard = self.lock.read();
            let contents = retained.owner.sheet.contents(&guard);
            let relative = match retained.relative {
                Some(style::stylesheets::CssRuleType::Style) => ParseRelative::ForNesting,
                Some(style::stylesheets::CssRuleType::Scope) => ParseRelative::ForScope,
                _ => ParseRelative::No,
            };
            let selector_parser = style::selector_parser::SelectorParser {
                stylesheet_origin: Origin::Author,
                namespaces: &contents.namespaces,
                url_data: &contents.url_data,
                for_supports_rule: false,
            };
            let mut input = cssparser::ParserInput::new(text);
            match cssparser::Parser::new(&mut input)
                .parse_entirely(|input| SelectorList::parse(&selector_parser, input, relative))
            {
                Ok(selectors) => selectors,
                Err(_) => return Ok(false),
            }
        };
        self.leases
            .lock()
            .map_err(|_| "native selector ledger poisoned")?
            .try_reserve(1)
            .map_err(|e| e.to_string())?;
        admission.checkpoint()?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "native selector ledger poisoned")?;
        // Caller holds exclusive graph borrow through parse/admission/commit.
        rule.write_with(&mut self.lock.write()).selectors = selectors;
        leases.push(lease);
        Ok(true)
    }
}

#[cfg(test)]
mod retained_selector_tests {
    use super::*;
    struct Admission;
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn selector_edits_preserve_original_rule_and_block_and_invalid_is_noop() {
        let budget = Admission;
        let mut graph = ImportGraph::parse(
            "@namespace svg url('http://www.w3.org/2000/svg'); div {color:red}",
            "https://example.test/root.css",
            false,
            8,
            SharedRwLock::new(),
            &budget,
        )
        .unwrap();
        let root = graph.root();
        let rule = graph.rule_at(&root, &[], 1, &budget).unwrap();
        let block = graph.declaration_block(&rule).unwrap();
        assert!(
            graph
                .set_rule_selector_text(&rule, "svg|rect", &budget)
                .unwrap()
        );
        assert!(Arc::ptr_eq(
            &block,
            &graph.declaration_block(&rule).unwrap()
        ));
        assert_eq!(
            rule.view().unwrap().selector_text.as_deref(),
            Some("svg|rect")
        );
        assert!(!graph.set_rule_selector_text(&rule, "[", &budget).unwrap());
        assert_eq!(
            rule.view().unwrap().selector_text.as_deref(),
            Some("svg|rect")
        );
    }
}

// Included inside native_import_graph.rs; all fields remain native-private.
pub struct PreparedImportCompletion {
    incumbent_occurrences: usize,
    incumbent_leases: usize,
    incumbent_charges: usize,
    lock: SharedRwLock,
    quirks: QuirksMode,
    max_depth: usize,
    id: OccurrenceId,
    parent: NativeSheet,
    rule: Arc<Locked<ImportRule>>,
    child: NativeSheet,
    expected_next: u64,
    next: u64,
    occurrences: Vec<NativeImport>,
    leases: Vec<LeaseArc<dyn Send + Sync>>,
    charges: Vec<LeaseArc<dyn Send + Sync>>,
    _overlap: LeaseArc<dyn Send + Sync>,
}
impl PreparedImportCompletion {
    pub fn child(&self) -> NativeSheet {
        self.child.clone()
    }
}
pub struct ImportCompletionContext {
    snapshot: ImportGraph,
}
impl ImportCompletionContext {
    pub fn prepare(
        &self,
        token: &CompletionToken,
        source: &str,
        final_url: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<Option<PreparedImportCompletion>, String> {
        self.snapshot
            .prepare_completion(token, source, final_url, admission)
    }
}
impl ImportGraph {
    /// Caller reserves metadata and Vec capacity outside its RefCell, then
    /// captures original handles without callbacks. Snapshot is parse-only.
    pub fn completion_context(
        &self,
        mut entries: Vec<NativeImport>,
    ) -> Result<ImportCompletionContext, String> {
        if entries.capacity() < self.occurrences.len() {
            return Err("completion snapshot capacity stale".into());
        }
        entries.extend(self.occurrences.entries.iter().cloned());
        Ok(ImportCompletionContext {
            snapshot: ImportGraph {
                lock: self.lock.clone(),
                root: self.root.clone(),
                leases: self.leases.clone(),
                next: self.next,
                font_revision: self.font_revision.clone(),
                font_descriptor_leases:self.font_descriptor_leases.clone(),
                quirks: self.quirks,
                max_depth: self.max_depth,
                occurrences: Occurrences {
                    entries,
                    charges: self.occurrences.charges.clone(),
                },
            },
        })
    }
    pub fn completion_context_count(&self) -> usize {
        self.occurrences.len()
    }
    pub fn prepare_completion(
        &self,
        token: &CompletionToken,
        source: &str,
        final_url: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<Option<PreparedImportCompletion>, String> {
        if !LeaseArc::ptr_eq(&token.parent.leases, &self.leases) {
            return Err("foreign completion token".into());
        }
        let entry = self
            .occurrences
            .get(&token.id)
            .ok_or("unknown completion occurrence")?;
        if !entry.parent.same_sheet(&token.parent) || !Arc::ptr_eq(&entry.rule, &token.rule) {
            return Err("stale completion identity".into());
        }
        if entry.state != ImportState::Pending {
            return Ok(None);
        }
        if entry.depth > self.max_depth {
            return Err("stylesheet import depth exceeded".into());
        }
        let depth = entry.depth.checked_add(1).ok_or("import depth overflow")?;
        let (sheet, parsed, source_lease) = Self::parse_sheet(
            source,
            final_url,
            self.quirks,
            &self.lock,
            entry.media.clone(),
            admission,
        )?;
        let child = NativeSheet {
            sheet,
            leases: self.leases.clone(),
        };
        let next = self
            .next
            .checked_add(u64::try_from(parsed.len()).map_err(|_| "import identity overflow")?)
            .ok_or("import identity exhausted")?;
        let occurrence_count = self
            .occurrences
            .len()
            .checked_add(parsed.len())
            .ok_or("occurrence capacity overflow")?;
        let old_lease_count = self
            .leases
            .lock()
            .map_err(|_| "import ledger poisoned")?
            .len();
        let old_charge_count = self
            .occurrences
            .charges
            .lock()
            .map_err(|_| "occurrence ledger poisoned")?
            .len();
        let lease_count = old_lease_count
            .checked_add(parsed.len())
            .and_then(|n| n.checked_add(1))
            .ok_or("lease capacity overflow")?;
        let charge_count = old_charge_count
            .checked_add(1)
            .ok_or("charge capacity overflow")?;
        let bytes = occurrence_count
            .checked_mul(std::mem::size_of::<NativeImport>())
            .and_then(|n| {
                n.checked_add(
                    lease_count.checked_mul(std::mem::size_of::<LeaseArc<dyn Send + Sync>>())?,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    charge_count.checked_mul(std::mem::size_of::<LeaseArc<dyn Send + Sync>>())?,
                )
            })
            .ok_or("completion metadata overflow")?;
        let overlap = admission.reserve(bytes)?;
        let retained = admission.reserve(bytes)?;
        let mut occurrences = Vec::new();
        occurrences
            .try_reserve_exact(occurrence_count)
            .map_err(|e| e.to_string())?;
        let mut leases = Vec::new();
        leases
            .try_reserve_exact(lease_count)
            .map_err(|e| e.to_string())?;
        let mut charges = Vec::new();
        charges
            .try_reserve_exact(charge_count)
            .map_err(|e| e.to_string())?;
        charges.push(retained);
        let guard = self.lock.read();
        let contents = child.sheet.contents(&guard);
        let rules = &contents.rules.read_with(&guard).0;
        let mut positions = rules
            .iter()
            .enumerate()
            .filter_map(|(i, rule)| matches!(rule, CssRule::Import(_)).then_some(i));
        for (offset, parsed) in parsed.into_iter().enumerate() {
            let index = positions
                .next()
                .ok_or("native import occurrence mismatch")?;
            let state = if matches!(
                parsed.rule.read_with(&guard).stylesheet,
                ImportSheet::Refused
            ) {
                ImportState::Refused
            } else {
                ImportState::Pending
            };
            leases.push(parsed.lease);
            occurrences.push(NativeImport {
                id: OccurrenceId(self.next + offset as u64),
                rule: parsed.rule,
                parent: child.clone(),
                child: None,
                media: parsed.media,
                state,
                index,
                depth,
            });
        }
        if positions.next().is_some() {
            return Err("native import occurrence mismatch".into());
        }
        leases.push(source_lease);
        drop(guard);
        admission.checkpoint()?;
        Ok(Some(PreparedImportCompletion {
            incumbent_occurrences: self.occurrences.len(),
            incumbent_leases: old_lease_count,
            incumbent_charges: old_charge_count,
            lock: self.lock.clone(),
            quirks: self.quirks,
            max_depth: self.max_depth,
            id: token.id,
            parent: token.parent.clone(),
            rule: token.rule.clone(),
            child,
            expected_next: self.next,
            next,
            occurrences,
            leases,
            charges,
            _overlap: overlap,
        }))
    }
    /// Err/false leave prepared and token caller-owned. No callbacks or
    /// allocation occur after the first vector move or rule association write.
    pub fn commit_prepared_completion(
        &mut self,
        prepared: &mut PreparedImportCompletion,
    ) -> Result<bool, String> {
        if self.next != prepared.expected_next {
            return Ok(false);
        }
        let entry = self
            .occurrences
            .get(&prepared.id)
            .ok_or("unknown prepared occurrence")?;
        if entry.state != ImportState::Pending
            || !entry.parent.same_sheet(&prepared.parent)
            || !Arc::ptr_eq(&entry.rule, &prepared.rule)
        {
            return Ok(false);
        }
        let mut leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        let mut charges = self
            .occurrences
            .charges
            .lock()
            .map_err(|_| "occurrence ledger poisoned")?;
        if prepared.occurrences.capacity() < self.occurrences.len() + prepared.occurrences.len()
            || prepared.leases.capacity() < leases.len() + prepared.leases.len()
            || prepared.charges.capacity() < charges.len() + prepared.charges.len()
        {
            return Ok(false);
        }
        let mut lock = self.lock.write();
        // Prepared staged entries go after incumbent entries; rotate in-place.
        let staged = prepared.occurrences.len();
        prepared.occurrences.append(&mut self.occurrences.entries);
        prepared.occurrences.rotate_left(staged);
        std::mem::swap(&mut self.occurrences.entries, &mut prepared.occurrences);
        prepared.leases.append(&mut leases);
        std::mem::swap(&mut *leases, &mut prepared.leases);
        prepared.charges.append(&mut charges);
        std::mem::swap(&mut *charges, &mut prepared.charges);
        let index = usize::try_from(prepared.id.0 - 1).expect("validated occurrence index");
        let entry = self
            .occurrences
            .entries
            .get_mut(index)
            .expect("validated original occurrence");
        entry.rule.write_with(&mut lock).stylesheet =
            ImportSheet::new(prepared.child.sheet.clone());
        entry.child = Some(prepared.child.clone());
        entry.state = ImportState::Complete;
        self.next = prepared.next;
        Ok(true)
    }
}

impl PreparedImportCompletion {
    /// Complete only newly parsed descendant rules. Never writes an incumbent
    /// graph rule; the entire branch is still invisible to cascade/CSSOM.
    pub fn prepare_descendant(
        &mut self,
        parent: &NativeSheet,
        rule_index: usize,
        source: Option<(&str, &str)>,
        admission: &dyn ImportAdmission,
    ) -> Result<Option<NativeSheet>, String> {
        let position = self
            .occurrences
            .iter()
            .position(|entry| entry.parent.same_sheet(parent) && entry.index == rule_index)
            .ok_or("source-tree import position absent")?;
        if self.occurrences[position].state != ImportState::Pending {
            return Ok(None);
        }
        if source.is_none() {
            self.occurrences[position].state = ImportState::Cancelled;
            return Ok(None);
        }
        let (source, final_url) = source.unwrap();
        let depth = self.occurrences[position].depth;
        // Existing loader depth is 8; graph configured limit is captured in
        // root preparation. Descendant depth must be checked by that limit.
        if depth > self.max_depth {
            return Err("stylesheet import depth exceeded".into());
        }
        let (sheet, parsed, lease) = ImportGraph::parse_sheet(
            source,
            final_url,
            self.quirks,
            &self.lock,
            self.occurrences[position].media.clone(),
            admission,
        )?;
        let child = NativeSheet {
            sheet,
            leases: self.parent.leases.clone(),
        };
        let next = self
            .next
            .checked_add(u64::try_from(parsed.len()).map_err(|_| "import identity overflow")?)
            .ok_or("import identity exhausted")?;
        let additional = parsed.len();
        let bytes = additional
            .checked_mul(std::mem::size_of::<NativeImport>())
            .and_then(|n| {
                n.checked_add(
                    (additional + 2)
                        .checked_mul(std::mem::size_of::<LeaseArc<dyn Send + Sync>>())?,
                )
            })
            .ok_or("descendant metadata overflow")?;
        let charge = admission.reserve(bytes)?;
        self.occurrences
            .try_reserve(
                additional
                    .checked_add(self.incumbent_occurrences)
                    .ok_or("descendant occurrence overflow")?,
            )
            .map_err(|e| e.to_string())?;
        self.leases
            .try_reserve(
                additional
                    .checked_add(1)
                    .and_then(|n| n.checked_add(self.incumbent_leases))
                    .ok_or("descendant lease overflow")?,
            )
            .map_err(|e| e.to_string())?;
        self.charges
            .try_reserve(
                self.incumbent_charges
                    .checked_add(1)
                    .ok_or("descendant charge overflow")?,
            )
            .map_err(|e| e.to_string())?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(additional)
            .map_err(|e| e.to_string())?;
        let guard = self.lock.read();
        let contents = child.sheet.contents(&guard);
        let mut positions = contents
            .rules
            .read_with(&guard)
            .0
            .iter()
            .enumerate()
            .filter_map(|(i, rule)| matches!(rule, CssRule::Import(_)).then_some(i));
        let next_depth = depth.checked_add(1).ok_or("import depth overflow")?;
        for (offset, parsed) in parsed.into_iter().enumerate() {
            let index = positions
                .next()
                .ok_or("native descendant import mismatch")?;
            let state = if matches!(
                parsed.rule.read_with(&guard).stylesheet,
                ImportSheet::Refused
            ) {
                ImportState::Refused
            } else {
                ImportState::Pending
            };
            staged.push((
                NativeImport {
                    id: OccurrenceId(self.next + offset as u64),
                    rule: parsed.rule,
                    parent: child.clone(),
                    child: None,
                    media: parsed.media,
                    state,
                    index,
                    depth: next_depth,
                },
                parsed.lease,
            ));
        }
        if positions.next().is_some() {
            return Err("native descendant import mismatch".into());
        }
        drop(guard);
        admission.checkpoint()?;
        for (entry, lease) in staged {
            self.occurrences.push(entry);
            self.leases.push(lease);
        }
        self.leases.push(lease);
        self.charges.push(charge);
        self.next = next;
        let entry = &mut self.occurrences[position];
        entry.rule.write_with(&mut self.lock.write()).stylesheet =
            ImportSheet::new(child.sheet.clone());
        entry.child = Some(child.clone());
        entry.state = ImportState::Complete;
        Ok(Some(child))
    }
}

#[cfg(test)]
mod prepared_completion_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Admission(AtomicBool);
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            if self.0.load(Ordering::SeqCst) {
                Err("forced failure".into())
            } else {
                Ok(())
            }
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            self.checkpoint()?;
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn prepared_child_keeps_token_on_failure_and_original_arc_on_commit() {
        let admission = Admission(AtomicBool::new(false));
        let mut graph = ImportGraph::parse(
            "@import 'a.css';",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &admission,
        )
        .unwrap();
        let id = OccurrenceId(1);
        let token = graph.begin_import(id).unwrap();
        admission.0.store(true, Ordering::SeqCst);
        assert!(
            graph
                .prepare_completion(
                    &token,
                    "@import 'b.css'; p{color:red}",
                    "https://example.test/a.css",
                    &admission
                )
                .is_err()
        );
        assert!(graph.occurrence(id).unwrap().child.is_none());
        admission.0.store(false, Ordering::SeqCst);
        let mut prepared = graph
            .prepare_completion(
                &token,
                "@import 'b.css'; p{color:red}",
                "https://example.test/a.css",
                &admission,
            )
            .unwrap()
            .unwrap();
        let child = prepared.child();
        assert!(graph.commit_prepared_completion(&mut prepared).unwrap());
        assert!(child.same_sheet(&graph.occurrence(id).unwrap().child.unwrap()));
        assert!(graph.begin_import(OccurrenceId(2)).is_some());
        assert!(!graph.commit_prepared_completion(&mut prepared).unwrap());
    }
    #[test]
    fn detached_ancestor_rejects_prepared_child_without_association() {
        let admission = Admission(AtomicBool::new(false));
        let mut graph = ImportGraph::parse(
            "@import 'a.css';",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &admission,
        )
        .unwrap();
        let token = graph.begin_import(OccurrenceId(1)).unwrap();
        let mut prepared = graph
            .prepare_completion(&token, "p{}", "https://example.test/a.css", &admission)
            .unwrap()
            .unwrap();
        graph.delete_rule(&graph.root(), 0).unwrap();
        assert!(!graph.commit_prepared_completion(&mut prepared).unwrap());
        assert!(graph.occurrence(OccurrenceId(1)).unwrap().child.is_none());
    }
}

pub struct PreparedRuleInsertion {
    sheet: NativeSheet,
    index: usize,
    rule: CssRule,
    expected_next: u64,
    expected_contents: Arc<style::stylesheets::StylesheetContents>,
    expected_rules: Vec<CssRule>,
    replacement_rules: Vec<CssRule>,
    occurrences: Vec<NativeImport>,
    leases: Vec<LeaseArc<dyn Send + Sync>>,
    charges: Vec<LeaseArc<dyn Send + Sync>>,
    next: u64,
    id: Option<OccurrenceId>,
}
fn insertion_capacity(current: usize, additional: usize) -> Result<usize, RuleMutationError> {
    current
        .checked_add(additional)
        .ok_or_else(|| RuleMutationError::InvalidContext("insertion capacity overflow".into()))
}
impl ImportCompletionContext {
    pub fn prepare_rule_insertion(
        &self,
        sheet: &NativeSheet,
        index: usize,
        text: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedRuleInsertion, RuleMutationError> {
        let graph = &self.snapshot;
        let invalid = |s: String| RuleMutationError::InvalidContext(s);
        admission.checkpoint().map_err(invalid)?;
        if !LeaseArc::ptr_eq(&sheet.leases, &graph.leases) {
            return Err(invalid("foreign graph sheet".into()));
        }
        let source = admission.reserve(text.len()).map_err(invalid)?;
        let loader = Loader {
            admission,
            imports: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        };
        let list = sheet.sheet.contents(&graph.lock.read()).rules.clone();
        let isolated = SharedRwLock::new();
        let (metadata_bytes, context_count, expected_contents) = {
            let guard = graph.lock.read();
            let contents = sheet.sheet.contents(&guard);
            (
                contents
                    .insertion_metadata_bytes()
                    .ok_or_else(|| invalid("insertion metadata overflow".into()))?,
                list.read_with(&guard).0.len(),
                sheet.sheet.contents.read_with(&guard).clone(),
            )
        };
        let metadata_charge = admission.reserve(metadata_bytes).map_err(invalid)?;
        let context_bytes = context_count
            .checked_mul(std::mem::size_of::<CssRule>())
            .ok_or_else(|| invalid("insertion context overflow".into()))?;
        let context_charge = admission.reserve(context_bytes).map_err(invalid)?;
        let mut parse_rules = Vec::new();
        parse_rules
            .try_reserve_exact(context_count)
            .map_err(|e| invalid(e.to_string()))?;
        for i in 0..context_count {
            admission.checkpoint().map_err(invalid)?;
            let guard = graph.lock.read();
            parse_rules.push(
                list.read_with(&guard)
                    .0
                    .get(i)
                    .ok_or_else(|| invalid("insertion context changed".into()))?
                    .clone(),
            );
        }
        let metadata = expected_contents
            .insertion_metadata_cancellable(&isolated, || admission.checkpoint())
            .map_err(invalid)?;
        if index > parse_rules.len() {
            return Err(RuleMutationError::IndexSize);
        }
        let parsed_rule = CssRule::parse(
            text,
            style::stylesheets::InsertRuleContext {
                rule_list: &parse_rules,
                index,
                containing_rule_types: style::stylesheets::CssRuleTypes::default(),
                parse_relative_rule_type: None,
            },
            &metadata,
            &isolated,
            Some(&loader),
            AllowImportRules::Yes,
        )?;
        if let Some(error) = loader.error.into_inner() {
            return Err(invalid(error));
        }
        use style::shared_lock::DeepCloneWithLock;
        let rule = parsed_rule.deep_clone_with_lock(&graph.lock, &isolated.read());
        let mut imports = loader.imports.into_inner();
        // Loader handles belong to isolated parser lock. Only transferred
        // original rule/media Arcs may enter the live graph occurrence index.
        if let CssRule::Import(transferred) = &rule
            && let Some(parsed) = imports.first_mut()
        {
            parsed.rule = transferred.clone();
            parsed.media = Arc::new(
                graph
                    .lock
                    .wrap(parsed.media.read_with(&isolated.read()).clone()),
            );
        }
        if imports.len() > 1 {
            return Err(invalid(
                "single inserted rule produced multiple imports".into(),
            ));
        }
        let next = graph
            .next
            .checked_add(imports.len() as u64)
            .ok_or_else(|| invalid("import identity overflow".into()))?;
        let depth = graph
            .occurrences
            .values()
            .find(|entry| {
                entry
                    .child
                    .as_ref()
                    .is_some_and(|child| child.same_sheet(sheet))
            })
            .map_or(Some(1), |entry| entry.depth.checked_add(1))
            .ok_or_else(|| invalid("import depth overflow".into()))?;
        let rule_count = parse_rules.len();
        let lease_count = graph
            .leases
            .lock()
            .map_err(|_| invalid("import ledger poisoned".into()))?
            .len();
        let charge_count = graph
            .occurrences
            .charges
            .lock()
            .map_err(|_| invalid("occurrence ledger poisoned".into()))?
            .len();
        let occurrence_count = graph
            .occurrences
            .len()
            .checked_add(imports.len())
            .ok_or_else(|| invalid("occurrence count overflow".into()))?;
        let rule_capacity = insertion_capacity(rule_count, 1)?;
        let lease_capacity = lease_count
            .checked_add(imports.len())
            .and_then(|n| n.checked_add(3))
            .ok_or_else(|| invalid("lease capacity overflow".into()))?;
        let charge_capacity = charge_count
            .checked_add(1)
            .ok_or_else(|| invalid("charge capacity overflow".into()))?;
        let combined_capacity = lease_capacity
            .checked_add(charge_capacity)
            .ok_or_else(|| invalid("combined capacity overflow".into()))?;
        let bytes = rule_capacity
            .checked_mul(2 * std::mem::size_of::<CssRule>())
            .and_then(|n| {
                n.checked_add(occurrence_count.checked_mul(std::mem::size_of::<NativeImport>())?)
            })
            .and_then(|n| {
                n.checked_add(
                    combined_capacity
                        .checked_mul(std::mem::size_of::<LeaseArc<dyn Send + Sync>>())?,
                )
            })
            .ok_or_else(|| invalid("insertion metadata overflow".into()))?;
        let allocation = admission.reserve(bytes).map_err(invalid)?;
        let expected_rules = parse_rules;
        let mut replacement_rules = Vec::new();
        replacement_rules
            .try_reserve_exact(rule_capacity)
            .map_err(|e| invalid(e.to_string()))?;
        let mut occurrences = Vec::new();
        occurrences
            .try_reserve_exact(occurrence_count)
            .map_err(|e| invalid(e.to_string()))?;
        let mut leases = Vec::new();
        leases
            .try_reserve_exact(lease_capacity)
            .map_err(|e| invalid(e.to_string()))?;
        let mut charges = Vec::new();
        charges
            .try_reserve_exact(charge_capacity)
            .map_err(|e| invalid(e.to_string()))?;
        for entry in &graph.occurrences.entries {
            admission.checkpoint().map_err(invalid)?;
            occurrences.push(entry.clone());
        }
        for entry in &mut occurrences {
            admission.checkpoint().map_err(invalid)?;
            if entry.parent.same_sheet(sheet) && entry.index != usize::MAX && entry.index >= index {
                entry.index = entry
                    .index
                    .checked_add(1)
                    .filter(|v| *v != usize::MAX)
                    .ok_or_else(|| invalid("import index overflow".into()))?;
            }
        }
        let mut id = None;
        for parsed in imports {
            let guard = graph.lock.read();
            let state = if matches!(
                parsed.rule.read_with(&guard).stylesheet,
                ImportSheet::Refused
            ) {
                ImportState::Refused
            } else {
                ImportState::Pending
            };
            id = Some(OccurrenceId(graph.next));
            leases.push(parsed.lease);
            occurrences.push(NativeImport {
                id: id.unwrap(),
                rule: parsed.rule,
                parent: sheet.clone(),
                child: None,
                media: parsed.media,
                state,
                index,
                depth,
            });
        }
        leases.push(source);
        leases.push(metadata_charge);
        leases.push(context_charge);
        charges.push(allocation);
        admission.checkpoint().map_err(invalid)?;
        Ok(PreparedRuleInsertion {
            sheet: sheet.clone(),
            index,
            rule,
            expected_next: graph.next,
            expected_contents,
            expected_rules,
            replacement_rules,
            occurrences,
            leases,
            charges,
            next,
            id,
        })
    }
}
impl ImportGraph {
    pub fn commit_rule_insertion(
        &mut self,
        prepared: &mut PreparedRuleInsertion,
    ) -> Result<Option<OccurrenceId>, RuleMutationError> {
        let invalid = |s: &str| RuleMutationError::InvalidContext(s.into());
        if self.next != prepared.expected_next {
            return Err(invalid("insertion graph changed"));
        }
        let list = prepared
            .sheet
            .sheet
            .contents(&self.lock.read())
            .rules
            .clone();
        if !Arc::ptr_eq(
            &prepared.expected_contents,
            prepared.sheet.sheet.contents.read_with(&self.lock.read()),
        ) {
            return Err(invalid("insertion contents changed"));
        }
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| invalid("import ledger poisoned"))?;
        let mut charges = self
            .occurrences
            .charges
            .lock()
            .map_err(|_| invalid("occurrence ledger poisoned"))?;
        let required_leases = leases
            .len()
            .checked_add(prepared.leases.len())
            .ok_or_else(|| invalid("final lease count overflow"))?;
        let required_charges = charges
            .len()
            .checked_add(prepared.charges.len())
            .ok_or_else(|| invalid("final charge count overflow"))?;
        if prepared.leases.capacity() < required_leases
            || prepared.charges.capacity() < required_charges
        {
            return Err(invalid("insertion capacity changed"));
        }
        let mut guard = self.lock.write();
        let rules = list.write_with(&mut guard);
        if rules.0.len() != prepared.expected_rules.len()
            || rules
                .0
                .iter()
                .zip(&prepared.expected_rules)
                .any(|(a, b)| !NativeRule::same_native_rule(a, b))
        {
            return Err(invalid("insertion rule list changed"));
        }
        prepared.replacement_rules.append(&mut rules.0);
        prepared
            .replacement_rules
            .insert(prepared.index, prepared.rule.clone());
        std::mem::swap(&mut rules.0, &mut prepared.replacement_rules);
        std::mem::swap(&mut self.occurrences.entries, &mut prepared.occurrences);
        prepared.leases.append(&mut leases);
        std::mem::swap(&mut *leases, &mut prepared.leases);
        prepared.charges.append(&mut charges);
        std::mem::swap(&mut *charges, &mut prepared.charges);
        self.next = prepared.next;
        Ok(prepared.id)
    }
}

#[cfg(test)]
mod prepared_insert_controls {
    use super::*;
    #[test]
    fn namespace_metadata_copy_cancels_during_prefix_walk() {
        let graph = ImportGraph::parse(
            "@namespace a url(urn:a); @namespace b url(urn:b); @namespace c url(urn:c);",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Allowed,
        )
        .unwrap();
        let contents = graph
            .root
            .sheet
            .contents
            .read_with(&graph.lock.read())
            .clone();
        let mut calls = 0usize;
        let result = contents.insertion_metadata_cancellable(&SharedRwLock::new(), || {
            calls += 1;
            if calls == 3 {
                Err("namespace walk cancelled".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(calls, 3);
        assert_eq!(contents.namespaces.prefixes.len(), 3);
        assert_eq!(graph.rule_count(&graph.root).unwrap(), 3);
    }
    struct Allowed;
    impl ImportAdmission for Allowed {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn isolated_namespace_parse_and_import_transfer_use_original_live_arcs() {
        let mut graph = ImportGraph::parse(
            "@namespace svg url(http://www.w3.org/2000/svg);",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Allowed,
        )
        .unwrap();
        let root = graph.root();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(graph.completion_context_count())
            .unwrap();
        let context = graph.completion_context(entries).unwrap();
        let mut selector = context
            .prepare_rule_insertion(&root, 1, "svg|circle{color:red}", &Allowed)
            .unwrap();
        graph.commit_rule_insertion(&mut selector).unwrap();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(graph.completion_context_count())
            .unwrap();
        let context = graph.completion_context(entries).unwrap();
        let mut imported = context
            .prepare_rule_insertion(&root, 0, "@import 'a.css' screen;", &Allowed)
            .unwrap();
        let id = graph.commit_rule_insertion(&mut imported).unwrap().unwrap();
        let original = graph.rule_at(&root, &[], 0, &Allowed).unwrap();
        assert!(graph
            .import_for_rule(&original)
            .unwrap()
            .same_rule(&graph.occurrence(id).unwrap()));
        assert_eq!(graph.occurrence(id).unwrap().media_text(), "screen");
    }
    #[test]
    fn changed_original_parse_context_rejects_insertion() {
        let mut graph = ImportGraph::parse(
            "p{}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Allowed,
        )
        .unwrap();
        let root = graph.root();
        let mut entries = Vec::new();
        entries.try_reserve_exact(0).unwrap();
        let context = graph.completion_context(entries).unwrap();
        let mut prepared = context
            .prepare_rule_insertion(&root, 0, "q{}", &Allowed)
            .unwrap();
        graph.insert_rule(&root, 0, "r{}", true, &Allowed).unwrap();
        assert!(graph.commit_rule_insertion(&mut prepared).is_err());
        assert_eq!(graph.rule_count(&root).unwrap(), 2);
    }
    #[test]
    fn admission_reenters_live_rule_mutation_without_publishing_prepared_rule() {
        struct Reentrant {
            lock: SharedRwLock,
            list: Arc<Locked<style::stylesheets::CssRules>>,
            calls: std::cell::Cell<usize>,
            armed: std::cell::Cell<bool>,
        }
        impl ImportAdmission for Reentrant {
            fn checkpoint(&self) -> Result<(), String> {
                let n = self.calls.get() + 1;
                self.calls.set(n);
                if self.armed.get() && n == 3 {
                    self.list.write_with(&mut self.lock.write()).0.clear();
                }
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                Ok(LeaseArc::new(()))
            }
        }
        let mut graph = ImportGraph::parse(
            "p{} q{}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Allowed,
        )
        .unwrap();
        let root = graph.root();
        let list = root.sheet.contents(&graph.lock.read()).rules.clone();
        let admission = Reentrant {
            lock: graph.lock.clone(),
            list,
            calls: std::cell::Cell::new(0),
            armed: std::cell::Cell::new(true),
        };
        let context = graph.completion_context(Vec::new()).unwrap();
        let result = context.prepare_rule_insertion(&root, 0, "r{}", &admission);
        if let Ok(mut prepared) = result {
            assert!(graph.commit_rule_insertion(&mut prepared).is_err());
        }
        assert_eq!(graph.rule_count(&root).unwrap(), 0);
        assert_eq!(graph.occurrences.len(), 0);
    }
    struct Cancelled;
    impl ImportAdmission for Cancelled {
        fn checkpoint(&self) -> Result<(), String> {
            Err("cancelled".into())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            self.checkpoint()?;
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn cancelled_prepare_does_not_publish_rule_or_occurrence() {
        struct Allowed;
        impl ImportAdmission for Allowed {
            fn checkpoint(&self) -> Result<(), String> {
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                Ok(LeaseArc::new(()))
            }
        }
        let graph = ImportGraph::parse(
            "p{}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Allowed,
        )
        .unwrap();
        let root = graph.root();
        let before = graph.rule_count(&root).unwrap();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(graph.completion_context_count())
            .unwrap();
        let context = graph.completion_context(entries).unwrap();
        assert!(context
            .prepare_rule_insertion(&root, 0, "@import 'a.css';", &Cancelled)
            .is_err());
        assert_eq!(graph.rule_count(&root).unwrap(), before);
        assert_eq!(graph.occurrences.len(), 0);
    }
    #[test]
    fn prospective_count_overflow_is_rejected() {
        assert!(insertion_capacity(usize::MAX, 1).is_err());
        assert_eq!(insertion_capacity(7, 1).unwrap(), 8);
    }
    #[test]
    fn cancellation_after_parse_during_rule_copy_preserves_graph() {
        struct Counter {
            calls: std::cell::Cell<usize>,
            stop: std::cell::Cell<usize>,
        }
        impl ImportAdmission for Counter {
            fn checkpoint(&self) -> Result<(), String> {
                let n = self.calls.get() + 1;
                self.calls.set(n);
                if n == self.stop.get() {
                    Err("late cancel".into())
                } else {
                    Ok(())
                }
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                Ok(LeaseArc::new(()))
            }
        }
        let admission = Counter {
            calls: std::cell::Cell::new(0),
            stop: std::cell::Cell::new(usize::MAX),
        };
        let graph = ImportGraph::parse(
            "p{} q{} r{} s{}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &admission,
        )
        .unwrap();
        let root = graph.root();
        let before = graph.rule_count(&root).unwrap();
        let next = graph.next;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(graph.completion_context_count())
            .unwrap();
        let context = graph.completion_context(entries).unwrap();
        admission.calls.set(0);
        let prepared = context
            .prepare_rule_insertion(&root, 0, "div{}", &admission)
            .unwrap();
        let total = admission.calls.get();
        drop(prepared);
        assert!(total > 2);
        admission.calls.set(0);
        admission.stop.set(total - 1);
        assert!(context
            .prepare_rule_insertion(&root, 0, "div{}", &admission)
            .is_err());
        assert_eq!(graph.rule_count(&root).unwrap(), before);
        assert_eq!(graph.next, next);
        assert_eq!(graph.occurrences.len(), 0);
    }
}

include!("font_face_descriptors.rs");
include!("font_demand_projection.rs");
include!("connected_font_face.rs");
include!("font_matching_axes.rs");
include!("constructed_font_face.rs");
impl ImportGraph {
    /// CSSPageRule selectorText edits only the original typed page selectors.
    pub fn set_page_selector_text(&mut self, retained: &NativeRule, text: &str, admission: &dyn ImportAdmission) -> Result<bool,String> {
        use style::parser::Parse;
        if !LeaseArc::ptr_eq(&retained.owner.leases,&self.leases) {return Err("foreign native page rule".into())}
        let CssRule::Page(rule)=&retained.rule else {return Err("native rule is not a page rule".into())};
        admission.checkpoint()?;
        let lease=admission.reserve(text.len())?;
        let selectors={
            let guard=self.lock.read();
            let base=&retained.owner.sheet.contents(&guard).url_data;
            let context=style::parser::ParserContext::new(Origin::Author,base,Some(style::stylesheets::CssRuleType::Page),style_traits::ParsingMode::DEFAULT,self.quirks,Default::default(),None,None,Default::default());
            if text.trim().is_empty() {style::stylesheets::PageSelectors::default()}
            else {
                let mut input=cssparser::ParserInput::new(text);
                match cssparser::Parser::new(&mut input).parse_entirely(|input|style::stylesheets::PageSelectors::parse(&context,input)) {Ok(value)=>value,Err(_)=>return Ok(false)}
            }
        };
        self.leases.lock().map_err(|_|"native page selector ledger poisoned")?.try_reserve(1).map_err(|error|error.to_string())?;
        admission.checkpoint()?;
        let mut leases=self.leases.lock().map_err(|_|"native page selector ledger poisoned")?;
        rule.write_with(&mut self.lock.write()).selectors=selectors;
        leases.push(lease);
        Ok(true)
    }
}
impl NativeRule {
    pub fn media_queries(&self) -> Result<Vec<String>, String> {
        use style_traits::ToCss;
        let CssRule::Media(rule) = &self.rule else { return Err("native rule has no media list".into()) };
        let guard = self.owner.sheet.shared_lock.read();
        Ok(rule.media_queries.read_with(&guard).media_queries.iter().map(|query| query.to_css_string()).collect())
    }
}
impl ImportGraph {
    /// Mutate the original retained MediaRule list, including detached rules.
    pub fn edit_rule_media(&mut self, retained: &NativeRule, edit: super::MediaListEdit<'_>, admission: &dyn ImportAdmission) -> Result<super::StylesheetMediaMutation, String> {
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) { return Err("foreign native rule".into()) }
        let CssRule::Media(rule) = &retained.rule else { return Err("native rule has no media list".into()) };
        let text = match edit { super::MediaListEdit::Replace(text)|super::MediaListEdit::Append(text)|super::MediaListEdit::Delete(text)=>text };
        admission.checkpoint()?;
        let lease = admission.reserve(text.len())?;
        let (base, mut media) = {
            let guard=self.lock.read();
            (retained.owner.sheet.contents(&guard).url_data.clone(), rule.media_queries.read_with(&guard).clone())
        };
        let mut context=style::parser::ParserContext::new(Origin::Author,&base,Some(style::stylesheets::CssRuleType::Media),style_traits::ParsingMode::DEFAULT,self.quirks,Default::default(),None,None,Default::default());
        let (changed,not_found)=super::media_lists::edit_native_media_list(&mut media,&mut context,edit);
        use style_traits::ToCss;
        let queries=media.media_queries.iter().map(|query|query.to_css_string()).collect();
        self.leases.lock().map_err(|_|"native media ledger poisoned")?.try_reserve(1).map_err(|error|error.to_string())?;
        admission.checkpoint()?;
        if changed { let mut leases=self.leases.lock().map_err(|_|"native media ledger poisoned")?; *rule.media_queries.write_with(&mut self.lock.write())=media; leases.push(lease); }
        Ok(super::StylesheetMediaMutation {queries,changed,not_found})
    }
}
