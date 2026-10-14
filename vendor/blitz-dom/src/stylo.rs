//! Enable the dom to participate in styling by servo
//!

use blitz_traits::node_id::NodeId;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::StyleThreading;
use crate::layout::damage::compute_layout_damage;
use crate::node::Node;
use crate::node::NodeData;
use markup5ever::{LocalName, LocalNameStaticSet, Namespace, NamespaceStaticSet, local_name};
use selectors::bloom::BLOOM_HASH_MASK;
use selectors::{
    Element, OpaqueElement,
    attr::{AttrSelectorOperation, NamespaceConstraint},
    matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode},
    sink::Push,
};
use style::CaseSensitivityExt;
use style::animation::AnimationSetKey;
use style::animation::AnimationState;
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::bloom::each_relevant_element_hash;
use style::color::AbsoluteColor;
use style::data::{ElementDataMut, ElementDataRef};
use style::global_style_data::STYLE_THREAD_POOL;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::ComputedValues;
use style::properties::{Importance, PropertyDeclaration};
use style::rule_tree::CascadeLevel;
use style::rule_tree::CascadeOrigin;
use style::selector_parser::PseudoElement;
use style::selector_parser::RestyleDamage;
use style::stylesheets::layer_rule::LayerOrder;
use style::stylesheets::scope_rule::ImplicitScopeRoot;
use style::values::AtomString;
use style::values::specified::NoCalcPercentage;
use style::{
    Atom,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext,
    },
    dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot},
    global_style_data::GLOBAL_STYLE_DATA,
    properties::PropertyDeclarationBlock,
    selector_parser::{NonTSPseudoClass, SelectorImpl},
    servo_arc::{Arc, ArcBorrow},
    shared_lock::{Locked, SharedRwLock, StylesheetGuards},
    thread_state::ThreadState,
    traversal::{DomTraversal, PerLevelTraversalData},
    traversal_flags::TraversalFlags,
    values::{AtomIdent, GenericAtomIdent},
};
use style_dom::ElementState;

use style::values::computed::text::TextAlign as StyloTextAlign;

/// Native style continuity keyed by the embedding document's stable node IDs.
/// No mirror pointers or tree storage survive a projection.
#[derive(Clone, Default)]
pub struct StyleContinuity {
    pub lock:Option<SharedRwLock>,
    pub(crate) styles: Vec<(u64, style::data::ElementStyles)>,
    pub(crate) sets: Vec<(u64, Option<PseudoElement>, style::animation::ElementAnimationSet)>,
    pub(crate) next_id:u64,
    pub active: bool,
}
impl std::fmt::Debug for StyleContinuity {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("StyleContinuity").field("nodes",&self.styles.len()).field("animation_sets",&self.sets.len()).field("active",&self.active).finish()}
}

impl crate::document::BaseDocument {
    pub fn restore_style_continuity(&mut self,previous:&StyleContinuity,ids:&std::collections::BTreeMap<u64,NodeId>){
        for (source,styles) in &previous.styles {
            let Some(node)=ids.get(source).and_then(|id|self.get_node(*id)).and_then(|node|node.as_element())else{continue};
            // SAFETY: the embedding caller owns this new mirror exclusively and
            // has not entered traversal. All restored nodes are fully restyled.
            let mut data=unsafe{node.ensure_data()};
            data.styles=styles.clone();
            data.hint.insert(RestyleHint::RESTYLE_SELF);
        }
        let mut sets=self.animations.sets.write();
        for (source,pseudo,set) in &previous.sets {
            if let Some(id)=ids.get(source){let mut set=set.clone();set.external_values=None;sets.insert(AnimationSetKey::new(OpaqueNode(id.as_u64() as usize),pseudo.clone()),set);}
        }
    }
    pub fn style_continuity(&self,ids:&std::collections::BTreeMap<u64,NodeId>)->StyleContinuity{
        let reverse=ids.iter().map(|(source,mirror)|(mirror.as_u64() as usize,*source)).collect::<std::collections::BTreeMap<_,_>>();
        let styles=ids.iter().filter_map(|(source,id)|self.get_node(*id)?.as_element()?.borrow_data().map(|data|(*source,data.styles.clone()))).collect();
        let sets=self.animations.sets.read().iter().filter_map(|(key,set)|Some((*reverse.get(&key.node.id())?,key.pseudo_element.clone(),set.clone()))).collect();
        StyleContinuity{lock:Some(self.guard.clone()),styles,sets,next_id:0,active:self.animations.sets.read().values().any(|set|set.needs_animation_ticks())}
    }

    /// Native layout adapter access to Stylo-owned pseudo styles. This does
    /// not create DOM nodes or run Blitz's box construction.
    pub fn resolved_pseudo_style(&self,node_id:NodeId,pseudo:&PseudoElement)->Option<Arc<style::properties::ComputedValues>> {
        let element=self.get_node(node_id)?.as_element()?;
        if pseudo.is_eager(){return element.borrow_data()?.styles.pseudos.get(pseudo).cloned();}
        let primary=self.resolved_primary_style(node_id)?;
        let mut ancestor=Some(element);
        while let Some(node)=ancestor {
            if node.primary_styles().is_some_and(|style|style.get_box().clone_display().is_none()){
                self.animations.sets.write().remove(&AnimationSetKey::new_for_pseudo(OpaqueNode(node_id.as_u64() as usize),pseudo.clone()));
                return None;
            }
            ancestor=node.parent_node().and_then(|parent|parent.as_element());
        }
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState {fn drop(&mut self){if self.0{style::thread_state::exit(ThreadState::LAYOUT);}}}
        let entered=!style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered{style::thread_state::enter(ThreadState::LAYOUT);}
        let _thread=LayoutThreadState(entered);
        let guard=self.guard.read();
        let computed=self.stylist.lazily_compute_pseudo_element_style(&StylesheetGuards{author:&guard,ua_or_user:&guard},element,pseudo,style::stylist::RuleInclusion::All,&primary,true,None)?;
        let key=AnimationSetKey::new_for_pseudo(OpaqueNode(node_id.as_u64() as usize),pseudo.clone());
        let mut set=self.animations.sets.write().remove(&key).unwrap_or_default();
        let shared=SharedStyleContext{traversal_flags:TraversalFlags::empty(),stylist:&self.stylist,options:GLOBAL_STYLE_DATA.options.clone(),guards:StylesheetGuards::same(&guard),visited_styles_enabled:false,animations:self.animations.clone(),current_time_for_animations:self.animation_sample_time,snapshot_map:&self.snapshots,registered_speculative_painters:&RegisteredPaintersImpl};
        let mut local=style::context::ThreadLocalStyleContext::new();local.bloom_filter.rebuild(element);
        let mut context=StyleContext{shared:&shared,thread_local:&mut local};
        let mut resolver=style::style_resolver::StyleResolverForElement::new(element,&mut context,style::stylist::RuleInclusion::All,style::style_resolver::PseudoElementResolution::Force);
        if !element.native_css_animations_rendered||computed.get_box().clone_display().is_none(){set.cancel_all_animations();}
        else{
            set.update_animations_for_new_style(element,&shared,&computed,&mut resolver);
            let previous=set.native_previous_style.clone();set.update_transitions_for_new_style(true,&shared,previous.as_ref(),&computed);
        }
        set.native_previous_style=Some(computed.clone());
        let mut result=computed;set.apply_active_animations(&shared,&mut result);
        self.animations.sets.write().insert(key,set);
        Some(result)
    }
    /// Resolve an eager child pseudo of an element-backed pseudo. The private
    /// mirror exists only during matching and never enters authored child lists.
    pub fn resolved_nested_pseudo_style(&mut self, node_id: NodeId, origin: &PseudoElement, pseudo: &PseudoElement) -> Option<Arc<ComputedValues>> {
        let primary = self.resolved_pseudo_style(node_id, origin)?;
        self.resolved_nested_pseudo_style_with_origin(node_id, pseudo, primary)
    }
    /// Resolve a nested pseudo using an already cascaded instance origin.
    pub fn resolved_nested_pseudo_style_with_origin(&mut self, node_id: NodeId, pseudo: &PseudoElement, primary: Arc<ComputedValues>) -> Option<Arc<ComputedValues>> {
        let name = markup5ever::QualName::new(None, markup5ever::Namespace::from("http://www.w3.org/1999/xhtml"), local_name!("div"));
        let mirror = self.create_node(NodeData::AnonymousBlock(Box::new(crate::node::ElementData::new(name, Vec::new()))));
        self.nodes[mirror].parent = Some(node_id);
        self.nodes[mirror].stylo_element_data_mut().ensure_init_mut().styles.primary = Some(primary.clone());
        let result = {
            struct LayoutThreadState(bool);
            impl Drop for LayoutThreadState { fn drop(&mut self) { if self.0 { style::thread_state::exit(ThreadState::LAYOUT); } } }
            let entered = !style::thread_state::get().contains(ThreadState::LAYOUT);
            if entered { style::thread_state::enter(ThreadState::LAYOUT); }
            let _thread = LayoutThreadState(entered);
            let guard = self.guard.read();
            let shared = SharedStyleContext {
                traversal_flags: TraversalFlags::empty(), stylist: &self.stylist,
                options: GLOBAL_STYLE_DATA.options.clone(), guards: StylesheetGuards::same(&guard),
                visited_styles_enabled: false, animations: self.animations.clone(),
                current_time_for_animations: self.animation_sample_time, snapshot_map: &self.snapshots,
                registered_speculative_painters: &RegisteredPaintersImpl,
            };
            let mut local = style::context::ThreadLocalStyleContext::new();
            let parent = &self.nodes[node_id];
            local.bloom_filter.rebuild(parent);
            local.bloom_filter.push(parent);
            let mut context = StyleContext { shared: &shared, thread_local: &mut local };
            style::style_resolver::StyleResolverForElement::new(
                &self.nodes[mirror], &mut context, style::stylist::RuleInclusion::All,
                style::style_resolver::PseudoElementResolution::Force,
            ).resolve_nested_pseudo_style(pseudo, primary)
        };
        self.changed_nodes.remove(&mirror);
        self.nodes.remove(mirror);
        result
    }
    /// Install script registrations directly into Stylo's document registry;
    /// they have script precedence over stylesheet @property declarations.
    pub fn register_custom_property(&mut self,name:&str,syntax:&str,inherits:bool,initial_value:Option<&str>,base_url:&str)->Result<(),String>{
        struct LayoutThreadState(bool);impl Drop for LayoutThreadState{fn drop(&mut self){if self.0{style::thread_state::exit(ThreadState::LAYOUT);}}}
        let entered=!style::thread_state::get().contains(ThreadState::LAYOUT);if entered{style::thread_state::enter(ThreadState::LAYOUT);}let _thread=LayoutThreadState(entered);
        let url=style::stylesheets::UrlExtraData::from(url::Url::parse(base_url).map_err(|_|"SyntaxError".to_owned())?);
        use style::stylist::RegisterCustomPropertyResult::*;
        match self.stylist.register_custom_property(&url,name,syntax,inherits,initial_value){SuccessfullyRegistered=>Ok(()),AlreadyRegistered=>Err("InvalidModificationError".into()),_=>Err("SyntaxError".into())}
    }
    /// Keep originating stylesheet scope alongside serialized anchor functions.
    pub fn anchor_reference_scopes(&self,node_id:NodeId,computed:&ComputedValues)->Vec<(String,String,Option<NodeId>)>{
        use style::values::{computed::{Inset,LengthPercentage,Margin,Size,MaxSize},generics::{position::TreeScoped,length::{GenericSize,GenericMaxSize,GenericMargin}},DashedIdent};
        type Reference=TreeScoped<DashedIdent>;
        fn length(value:&LengthPercentage,out:&mut Vec<Reference>){if let style::values::computed::length_percentage::Unpacked::Calc(calc)=value.unpack(){out.extend(calc.anchor_references());}}
        fn inset(value:&Inset,out:&mut Vec<Reference>){match value{
            Inset::AnchorFunction(value)=>{out.push(value.target_element.clone());if let Some(fallback)=value.fallback.as_ref(){inset(fallback,out);}},
            Inset::AnchorSizeFunction(value)=>{out.push(value.target_element.clone());if let Some(fallback)=value.fallback.as_ref(){inset(fallback,out);}},
            Inset::LengthPercentage(value)|Inset::AnchorContainingCalcFunction(value)=>length(value,out),_=>{},
        }}
        fn size(value:&Size,out:&mut Vec<Reference>){match value{
            GenericSize::AnchorSizeFunction(value)=>{out.push(value.target_element.clone());if let Some(fallback)=value.fallback.as_ref(){size(fallback,out);}},
            GenericSize::LengthPercentage(value)|GenericSize::AnchorContainingCalcFunction(value)|GenericSize::FitContentFunction(value)=>length(&value.0,out),_=>{},
        }}
        fn max_size(value:&MaxSize,out:&mut Vec<Reference>){match value{
            GenericMaxSize::AnchorSizeFunction(value)=>{out.push(value.target_element.clone());if let Some(fallback)=value.fallback.as_ref(){max_size(fallback,out);}},
            GenericMaxSize::LengthPercentage(value)|GenericMaxSize::AnchorContainingCalcFunction(value)|GenericMaxSize::FitContentFunction(value)=>length(&value.0,out),_=>{},
        }}
        fn margin(value:&Margin,out:&mut Vec<Reference>){match value{
            GenericMargin::AnchorSizeFunction(value)=>{out.push(value.target_element.clone());if let Some(fallback)=value.fallback.as_ref(){margin(fallback,out);}},
            GenericMargin::LengthPercentage(value)|GenericMargin::AnchorContainingCalcFunction(value)=>length(value,out),_=>{},
        }}
        let Some(element)=self.get_node(node_id).and_then(|node|node.as_element())else{return Vec::new()};
        let scope=|level:CascadeLevel|level.get_shadow_root_for_scoped(element).map(|root|root.id);
        let mut result=vec![("anchor-name".into(),String::new(),scope(computed.get_box().clone_anchor_name().scope)),("anchor-scope".into(),String::new(),scope(computed.get_box().clone_anchor_scope().scope)),("position-anchor".into(),String::new(),scope(computed.get_position().clone_position_anchor().scope))];
        let position=computed.get_position();let margins=computed.get_margin();
        macro_rules! collect{($name:literal,$value:expr,$collect:ident)=>{{let mut references=Vec::new();$collect(&$value,&mut references);for reference in references{result.push(($name.into(),reference.value.0.to_string(),scope(reference.scope)));}}};}
        collect!("top",position.clone_top(),inset);collect!("right",position.clone_right(),inset);collect!("bottom",position.clone_bottom(),inset);collect!("left",position.clone_left(),inset);
        collect!("width",position.clone_width(),size);collect!("height",position.clone_height(),size);collect!("min-width",position.clone_min_width(),size);collect!("min-height",position.clone_min_height(),size);collect!("max-width",position.clone_max_width(),max_size);collect!("max-height",position.clone_max_height(),max_size);
        collect!("margin-top",margins.clone_margin_top(),margin);collect!("margin-right",margins.clone_margin_right(),margin);collect!("margin-bottom",margins.clone_margin_bottom(),margin);collect!("margin-left",margins.clone_margin_left(),margin);
        result
    }
    /// Native anchor fallback cascade, including tree-scoped @position-try rules.
    pub fn resolved_position_tries(&self, node_id: NodeId, cb_wm:style::logical_geometry::WritingMode) -> Vec<Arc<ComputedValues>> {
        let Some(original)=self.resolved_primary_style(node_id) else{return Vec::new()};
        self.resolved_position_tries_for_style(node_id,&original,cb_wm)
    }
    /// Pseudos retain their own matched fallback declarations and cascade scope.
    pub fn resolved_position_tries_for_style(&self,node_id:NodeId,original:&ComputedValues,cb_wm:style::logical_geometry::WritingMode)->Vec<Arc<ComputedValues>> {
        self.resolved_position_tries_with_parent(node_id, original, cb_wm, None)
    }
    /// Use the owning instance or pseudo inheritance parent while retaining source rule scope.
    pub fn resolved_position_tries_with_parent(&self,node_id:NodeId,original:&ComputedValues,cb_wm:style::logical_geometry::WritingMode,parent:Option<&ComputedValues>)->Vec<Arc<ComputedValues>> {
        let Some(element)=self.get_node(node_id).and_then(|node|node.as_element()) else{return Vec::new()};
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState {fn drop(&mut self){if self.0{style::thread_state::exit(ThreadState::LAYOUT);}}}
        let entered=!style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered{style::thread_state::enter(ThreadState::LAYOUT);}
        let _thread=LayoutThreadState(entered);let guard=self.guard.read();
        let fallbacks=original.get_position().clone_position_try_fallbacks();
        fallbacks.value.0.iter().filter_map(|item|self.stylist.resolve_position_try_native(
            &original,&StylesheetGuards::same(&guard),fallbacks.scope,element,item,cb_wm,parent)).collect()
    }
    /// Recompute inherited values beneath a first-line formatting parent using
    /// the original native cascade. Authored inline declarations keep their
    /// precedence; reset properties remain those of the ordinary element.
    pub fn reparent_first_line_style(&self, node_id: NodeId, parent: &ComputedValues) -> Option<Arc<ComputedValues>> {
        let original = self.resolved_primary_style(node_id)?;
        self.reparent_first_line_style_from(node_id, &original, parent)
    }
    /// Apply first-line inheritance to an already resolved instance style.
    pub fn reparent_first_line_style_from(&self, node_id: NodeId, original: &ComputedValues, parent: &ComputedValues) -> Option<Arc<ComputedValues>> {
        let element = self.get_node(node_id)?.as_element()?;
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState { fn drop(&mut self) { if self.0 { style::thread_state::exit(ThreadState::LAYOUT); } } }
        let entered = !style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered { style::thread_state::enter(ThreadState::LAYOUT); }
        let _thread = LayoutThreadState(entered);
        let guard = self.guard.read();
        Some(self.stylist.cascade_style_and_visited(
            Some(element), None, &style::context::CascadeInputs::new_from_style(original),
            &StylesheetGuards::same(&guard), Some(parent), Some(parent),
            style::properties::FirstLineReparenting::Yes { style_to_reparent: original },
            &Default::default(), None, &mut Default::default(), &mut Default::default(),
        ))
    }
    /// Match a pseudo on its original source element, but cascade beneath the
    /// instance's primary style. This also handles sources whose original tree
    /// was pruned by display:none, as is common for SVG definitions.
    pub fn resolved_instance_pseudo_style(&self, node_id: NodeId, pseudo: &PseudoElement, primary: Arc<ComputedValues>) -> Option<Arc<ComputedValues>> {
        let element = self.get_node(node_id)?.as_element()?;
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState { fn drop(&mut self) { if self.0 { style::thread_state::exit(ThreadState::LAYOUT); } } }
        let entered = !style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered { style::thread_state::enter(ThreadState::LAYOUT); }
        let _thread = LayoutThreadState(entered);
        let guard = self.guard.read();
        if !pseudo.is_eager() {
            return self.stylist.lazily_compute_pseudo_element_style(
                &StylesheetGuards::same(&guard), element, pseudo,
                style::stylist::RuleInclusion::All, &primary, true, None,
            );
        }
        let shared = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(), stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(), guards: StylesheetGuards::same(&guard),
            visited_styles_enabled: false, animations: self.animations.clone(),
            current_time_for_animations: self.animation_sample_time, snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };
        let mut local = style::context::ThreadLocalStyleContext::new();
        local.bloom_filter.rebuild(element);
        let mut context = StyleContext { shared: &shared, thread_local: &mut local };
        style::style_resolver::StyleResolverForElement::new(
            element, &mut context, style::stylist::RuleInclusion::All,
            style::style_resolver::PseudoElementResolution::Force,
        ).resolve_nested_pseudo_style(pseudo, primary)
    }
    /// Recompute a referenced element under an SVG instance parent while
    /// retaining the original selector matches, cascade origins and priority.
    pub fn reparent_element_style(&self, node_id: NodeId, parent: &ComputedValues) -> Option<Arc<ComputedValues>> {
        let original = self.resolved_primary_style(node_id)?;
        self.reparent_text_pseudo_style(node_id, &original, parent)
    }
    /// Recascade a previously matched pseudo beneath its formatting parent.
    /// Used by first-letter inheritance after first-line reparenting.
    pub fn reparent_text_pseudo_style(&self, node_id: NodeId, original: &ComputedValues, parent: &ComputedValues) -> Option<Arc<ComputedValues>> {
        let element = self.get_node(node_id)?.as_element()?;
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState { fn drop(&mut self) { if self.0 { style::thread_state::exit(ThreadState::LAYOUT); } } }
        let entered = !style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered { style::thread_state::enter(ThreadState::LAYOUT); }
        let _thread = LayoutThreadState(entered);
        let guard = self.guard.read();
        Some(self.stylist.cascade_style_and_visited(
            Some(element), original.pseudo().as_ref(), &style::context::CascadeInputs::new_from_style(original),
            &StylesheetGuards::same(&guard), Some(parent), Some(parent),
            style::properties::FirstLineReparenting::No,
            &Default::default(), None, &mut Default::default(), &mut Default::default(),
        ))
    }
    /// Resolve a primary style even when normal traversal pruned this element
    /// beneath display:none. Call after resolve_stylist; this does not create boxes
    /// or change the element's display value.
    pub fn resolved_primary_style(
        &self,
        node_id: NodeId,
    ) -> Option<Arc<style::properties::ComputedValues>> {
        let element = self.get_node(node_id)?.as_element()?;
        if let Some(computed) = element.primary_styles() {
            return Some(Arc::clone(&computed));
        }
        struct LayoutThreadState(bool);
        impl Drop for LayoutThreadState {
            fn drop(&mut self) {
                if self.0 {
                    style::thread_state::exit(ThreadState::LAYOUT);
                }
            }
        }
        let entered = !style::thread_state::get().contains(ThreadState::LAYOUT);
        if entered {
            style::thread_state::enter(ThreadState::LAYOUT);
        }
        let _thread_state = LayoutThreadState(entered);
        let guard = self.guard.read();
        let shared = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards: StylesheetGuards {
                author: &guard,
                ua_or_user: &guard,
            },
            visited_styles_enabled: false,
            animations: self.animations.clone(),
            current_time_for_animations: self.animation_sample_time,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };
        let mut local = style::context::ThreadLocalStyleContext::new();
        let mut context = StyleContext {
            shared: &shared,
            thread_local: &mut local,
        };
        let resolved = style::traversal::resolve_style(
            &mut context,
            element,
            style::stylist::RuleInclusion::All,
            None,
            None,
        );
        Some(resolved.primary().clone())
    }

    pub fn resolve_stylist(&mut self, now: f64) {
        let _ = self.resolve_stylist_controlled(now, None);
    }

    /// Returns false after all scoped workers join if the private traversal was
    /// cancelled. An incomplete mirror must be discarded, never reused.
    pub fn resolve_stylist_controlled(&mut self, now: f64, cancelled: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>) -> bool {
        if cancelled.as_ref().is_some_and(|check| check()) { return false; }
        self.animation_sample_time=now;
        style::thread_state::enter(ThreadState::LAYOUT);
        // Callback is Rust owner state only. Early returns release borrowed
        // style/animation guards normally; this mirror is discarded.
        macro_rules! terminal_checkpoint {
            () => {
                if cancelled.as_ref().is_some_and(|check| check()) {
                    style::thread_state::exit(ThreadState::LAYOUT);
                    return false;
                }
            };
        }

        let guard = &self.guard;
        let guards = StylesheetGuards {
            author: &guard.read(),
            ua_or_user: &guard.read(),
        };

        let root = TDocument::as_node(&&self.nodes[self.root_node_id])
            .first_element_child()
            .unwrap()
            .as_element()
            .unwrap();

        let css_rules_changed=self.stylist.stylesheets_have_changed();
        self.stylist
            .flush(&guards)
            .process_style(root, Some(&self.snapshots));

        // Mark actively animating nodes as dirty
        let mut sets = self.animations.sets.write();
        for (key, set) in sets.iter_mut() {
            terminal_checkpoint!();
            let node_id = NodeId::from_u64(key.node.id() as u64);

            // Drop animations belonging to nodes that are no longer in the
            // document. A removed element is never restyled, so it would never
            // get a chance to cancel its own animations; an infinite animation
            // would then keep `has_active_animations` set forever and force a
            // redraw every frame. Emptying the set here lets the `retain` below
            // discard it so the flag can clear on this same pass.
            let in_document = self
                .nodes
                .get(node_id)
                .is_some_and(|node| node.flags.is_in_document());
            if !in_document {
                set.animations.clear();
                set.transitions.clear();
                continue;
            }

            self.nodes[node_id].set_restyle_hint(RestyleHint::RESTYLE_SELF);

            for animation in set.animations.iter_mut() {
                terminal_checkpoint!();
                if animation.state == AnimationState::Pending && animation.started_at <= now {
                    animation.state = AnimationState::Running;
                }
                animation.iterate_if_necessary(now);

                if animation.state == AnimationState::Running && animation.has_ended(now) {
                    animation.state = AnimationState::Finished;
                }
            }

            for transition in set.transitions.iter_mut() {
                terminal_checkpoint!();
                if transition.state == AnimationState::Pending && transition.start_time <= now {
                    transition.state = AnimationState::Running;
                }
                if transition.state == AnimationState::Running && transition.has_ended(now) {
                    transition.state = AnimationState::Finished;
                }
            }
        }
        drop(sets);

        // Build the style context used by the style traversal
        let context = SharedStyleContext {
            traversal_flags: if css_rules_changed{TraversalFlags::ForCSSRuleChanges}else{TraversalFlags::empty()},
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: self.animations.clone(),
            current_time_for_animations: now,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };

        // components/layout_2020/lib.rs:983
        let root = self.root_element();
        // dbg!(root);
        let token = RecalcStyle::pre_traverse(root, &context);

        let mut nodes_needing_style_image_flush = Vec::new();
        if token.should_traverse() {
            // Style the elements, resolving their data
            let mut traverser = RecalcStyle::new(context);
            traverser.cancelled = cancelled.clone();
            // `Sequential` bypasses Stylo's global pool. See `StyleThreading`.
            let pool_guard = matches!(self.style_threading, StyleThreading::Parallel)
                .then(|| STYLE_THREAD_POOL.pool());
            let rayon_pool = pool_guard.as_ref().and_then(|g| g.as_ref());
            style::driver::traverse_dom(&traverser, token, rayon_pool);
            // traverse_dom's scoped pool has joined; no worker can retain this
            // private mirror after cancellation is returned to its owner.
            if traverser.cancelled() {
                style::thread_state::exit(ThreadState::LAYOUT);
                return false;
            }
            nodes_needing_style_image_flush =
                std::mem::take(traverser.nodes_needing_style_image_flush.get_mut().unwrap());
        }
        if cancelled.as_ref().is_some_and(|check| check()) {
            style::thread_state::exit(ThreadState::LAYOUT);
            return false;
        }
        self.pending_style_image_nodes
            .extend(nodes_needing_style_image_flush);

        for opaque in self.snapshots.keys() {
            terminal_checkpoint!();
            let id = NodeId::from_u64(opaque.id() as u64);
            if let Some(node) = self.nodes.get_mut(id) {
                node.set_has_snapshot(false);
            }
        }
        self.snapshots.clear();

        let mut sets = self.animations.sets.write();
        for set in sets.values_mut() {
            terminal_checkpoint!();
            let mut kept = 0;
            for index in 0..set.animations.len() {
                terminal_checkpoint!();
                if set.animations[index].state != AnimationState::Canceled {
                    set.animations.swap(kept, index);
                    kept += 1;
                }
            }
            set.animations.truncate(kept);
            let mut kept = 0;
            for index in 0..set.transitions.len() {
                terminal_checkpoint!();
                if set.transitions[index].state != AnimationState::Canceled {
                    set.transitions.swap(kept, index);
                    kept += 1;
                }
            }
            set.transitions.truncate(kept);
            for animation in set.animations.iter_mut() {
                terminal_checkpoint!();
                animation.is_new = false;
            }
            for transition in set.transitions.iter_mut() {
                terminal_checkpoint!();
                transition.is_new = false;
            }
        }
        let mut empty = Vec::new();
        for (key, state) in sets.iter() {
            terminal_checkpoint!();
            if state.is_empty() { empty.push(key.clone()); }
        }
        for key in empty {
            terminal_checkpoint!();
            sets.remove(&key);
        }
        let mut active = false;
        for state in sets.values() {
            terminal_checkpoint!();
            for animation in &state.animations {
                terminal_checkpoint!();
                active |= matches!(animation.state, AnimationState::Running | AnimationState::Pending);
            }
            for transition in &state.transitions {
                terminal_checkpoint!();
                active |= matches!(transition.state, AnimationState::Running | AnimationState::Pending);
            }
        }
        self.has_active_animations = active;

        if cancelled.as_ref().is_some_and(|check| check()) {
            style::thread_state::exit(ThreadState::LAYOUT);
            return false;
        }
        // Maybe run garbage collection. Stylo has internal to determine whether to run or not.
        self.stylist.rule_tree().maybe_gc();

        style::thread_state::exit(ThreadState::LAYOUT);
        true
    }
}

/// A handle to a node that Servo's style traits are implemented against
///
/// Since BlitzNodes are not persistent (IE we don't keep the pointers around between frames), we choose to just implement
/// the tree structure in the nodes themselves, and temporarily give out pointers during the layout phase.
type BlitzNode<'a> = &'a Node;

#[cfg(test)]
mod controlled_traversal_tests {
    use super::*;
    use crate::{QualName, Namespace, LocalName};
    use std::sync::{Arc as StdArc, atomic::{AtomicUsize, Ordering}};
    #[test]
    fn parallel_controlled_resolve_joins_before_private_mirror_drop() {
        let mut document = crate::BaseDocument::new(crate::DocumentConfig {
            style_threading: crate::StyleThreading::Parallel,
            ..Default::default()
        });
        {
            let mut mutator = document.mutate();
            let root = mutator.create_element(QualName::new(None, Namespace::from("http://www.w3.org/1999/xhtml"), LocalName::from("html")), Vec::new());
            let parent = mutator.doc.root_node_id;
            mutator.append_children(parent, &[root]);
            for _ in 0..4096 {
                let child = mutator.create_element(QualName::new(None, Namespace::from("http://www.w3.org/1999/xhtml"), LocalName::from("div")), Vec::new());
                mutator.append_children(root, &[child]);
            }
        }
        let calls = StdArc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        assert!(!document.resolve_stylist_controlled(0., Some(StdArc::new(move || seen.fetch_add(1, Ordering::Relaxed) >= 32))));
        let joined_count = calls.load(Ordering::Relaxed);
        drop(document);
        assert_eq!(calls.load(Ordering::Relaxed), joined_count);
        assert!(joined_count >= 33);
    }
}

impl<'a> TDocument for BlitzNode<'a> {
    type ConcreteNode = BlitzNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn is_html_document(&self) -> bool {
        true
    }

    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        self.guard()
    }
}

impl NodeInfo for BlitzNode<'_> {
    fn is_element(&self) -> bool {
        Node::is_element(self)
    }

    fn is_text_node(&self) -> bool {
        Node::is_text_node(self)
    }
}

impl<'a> TShadowRoot for BlitzNode<'a> {
    type ConcreteNode = BlitzNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        self.with(self.shadow_host.expect("native shadow root host"))
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        self.shadow_style.as_ref()?.cascade.as_deref()
    }
}

// components/styleaapper.rs:
impl<'a> TNode for BlitzNode<'a> {
    type ConcreteElement = BlitzNode<'a>;
    type ConcreteDocument = BlitzNode<'a>;
    type ConcreteShadowRoot = BlitzNode<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.parent.map(|id| self.with(id))
    }

    fn first_child(&self) -> Option<Self> {
        self.children.first().map(|id| self.with(*id))
    }

    fn last_child(&self) -> Option<Self> {
        self.children.last().map(|id| self.with(*id))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.backward(1)
    }

    fn next_sibling(&self) -> Option<Self> {
        self.forward(1)
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        // Walk up the (layout-)parent chain to the root Document node.
        let mut node = *self;
        while let Some(parent_id) = node.parent {
            node = node.with(parent_id);
        }
        node
    }

    fn is_in_document(&self) -> bool {
        true
    }

    // I think this is the same as parent_node only in the cases when the direct parent is not a real element, forcing us
    // to travel upwards
    //
    // For the sake of this demo, we're just going to return the parent node ann
    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        if let Some(slot)=self.assigned_slot{return Some(self.with(slot));}
        self.parent_node().and_then(|node|node.shadow_host.map(|host|node.with(host)).or_else(||node.as_element()))
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.id.as_u64() as usize)
    }

    fn debug_id(self) -> usize {
        self.id.as_u64() as usize
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        match self.data {
            NodeData::Element { .. } => Some(self),
            _ => None,
        }
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        match self.data {
            NodeData::Document(_) if self.shadow_host.is_none() => Some(self),
            _ => None,
        }
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        self.shadow_host.map(|_|*self)
    }
}

impl selectors::Element for BlitzNode<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> selectors::OpaqueElement {
        crate::shadow_style::opaque(self.selector_identity)
    }

    fn parent_element(&self) -> Option<Self> {
        self.parent_node().and_then(|node|node.as_element())
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        self.parent_node().is_some_and(|node|node.shadow_host.is_some())
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        self.containing_shadow.and_then(|root|self.with(root).shadow_host).map(|host|self.with(host))
    }

    fn assigned_slot(&self)->Option<Self>{self.assigned_slot.map(|slot|self.with(slot))}

    fn is_pseudo_element(&self) -> bool {
        matches!(self.data, NodeData::AnonymousBlock(_))
    }

    // These methods are implemented naively since we only threaded real nodes and not fake nodes
    // we should try and use `find` instead of this foward/backward stuff since its ugly and slow
    fn prev_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.backward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.forward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

    fn first_element_child(&self) -> Option<Self> {
        let mut children = self.dom_children();
        children.find(|child| child.is_element())
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.has_namespace(&markup5ever::Namespace::from("http://www.w3.org/1999/xhtml"))
    }

    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.data.is_element_with_tag_name(local_name)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.element_data().expect("Not an element").name.ns == *ns
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name() && self.namespace() == other.namespace()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&GenericAtomIdent<NamespaceStaticSet>>,
        local_name: &GenericAtomIdent<LocalNameStaticSet>,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        self.data.attrs().is_some_and(|attributes|attributes.iter().any(|attribute|{
            attribute.name.local==local_name.0&&(match ns{NamespaceConstraint::Any=>true,NamespaceConstraint::Specific(ns)=>attribute.name.ns==ns.0})&&operation.eval_str(&attribute.value)
        }))
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &<Self::Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pseudo_class {
            NonTSPseudoClass::Active => self.element_state().contains(ElementState::ACTIVE),
            NonTSPseudoClass::AnyLink => self
                .element_state()
                .intersects(ElementState::VISITED_OR_UNVISITED),
            NonTSPseudoClass::Checked => self.element_state().contains(ElementState::CHECKED),
            NonTSPseudoClass::Valid => self.element_state().contains(ElementState::VALID),
            NonTSPseudoClass::Invalid => self.element_state().contains(ElementState::INVALID),
            NonTSPseudoClass::Granted => self.native_permission_granted,
            NonTSPseudoClass::Fullscreen => self.native_fullscreen,
            NonTSPseudoClass::Paused => self.native_media_state&2!=0,
            NonTSPseudoClass::Playing => self.native_media_state&4!=0,
            NonTSPseudoClass::Seeking => self.native_media_state&8!=0,
            NonTSPseudoClass::Buffering => self.native_media_state&16!=0,
            NonTSPseudoClass::Stalled => self.native_media_state&32!=0,
            NonTSPseudoClass::InterestSource => self.native_interest_source,
            NonTSPseudoClass::InterestTarget => self.native_interest_target,
            NonTSPseudoClass::Defined => self.element_state().contains(ElementState::DEFINED),
            NonTSPseudoClass::Disabled => self.element_state().contains(ElementState::DISABLED),
            NonTSPseudoClass::Enabled => self.element_state().contains(ElementState::ENABLED),
            NonTSPseudoClass::Focus => self.element_state().contains(ElementState::FOCUS),
            NonTSPseudoClass::FocusWithin => {
                self.element_state().contains(ElementState::FOCUS_WITHIN)
            }
            NonTSPseudoClass::FocusVisible => {
                self.element_state().contains(ElementState::FOCUSRING)
            }
            NonTSPseudoClass::Hover => self.element_state().contains(ElementState::HOVER),
            NonTSPseudoClass::Indeterminate => self.element_state().contains(ElementState::INDETERMINATE),
            NonTSPseudoClass::Dir(ref direction) => self.native_direction.eq_ignore_ascii_case(direction),
            NonTSPseudoClass::Lang(ref ranges) => self.match_element_lang(None,ranges),
            NonTSPseudoClass::CustomState(ref state) => self.native_custom_states.contains(&state.0),
            NonTSPseudoClass::Link => self.element_state().contains(ElementState::UNVISITED),
            NonTSPseudoClass::PlaceholderShown => self.element_state().contains(ElementState::PLACEHOLDER_SHOWN),
            NonTSPseudoClass::ReadWrite => self.element_state().contains(ElementState::READWRITE),
            NonTSPseudoClass::ReadOnly => self.element_state().contains(ElementState::READONLY),
            NonTSPseudoClass::ServoNonZeroBorder => false,
            NonTSPseudoClass::Target => self.element_state().contains(ElementState::URLTARGET),
            NonTSPseudoClass::Visited => false,
            NonTSPseudoClass::Autofill => false,
            NonTSPseudoClass::Default => self.element_state().contains(ElementState::DEFAULT),

            NonTSPseudoClass::InRange => self.element_state().contains(ElementState::INRANGE),
            NonTSPseudoClass::Modal => self.element_state().contains(ElementState::MODAL),
            NonTSPseudoClass::Open => self.element_state().contains(ElementState::OPEN),
            NonTSPseudoClass::Optional => self.element_state().contains(ElementState::OPTIONAL_),
            NonTSPseudoClass::OutOfRange => self.element_state().contains(ElementState::OUTOFRANGE),
            NonTSPseudoClass::PopoverOpen => self.element_state().contains(ElementState::POPOVER_OPEN),
            NonTSPseudoClass::Required => self.element_state().contains(ElementState::REQUIRED),
            NonTSPseudoClass::UserInvalid => self.element_state().contains(ElementState::USER_INVALID),
            NonTSPseudoClass::UserValid => self.element_state().contains(ElementState::USER_VALID),
            NonTSPseudoClass::MozMeterOptimum => false,
            NonTSPseudoClass::MozMeterSubOptimum => false,
            NonTSPseudoClass::MozMeterSubSubOptimum => false,
        }
    }

    fn match_pseudo_element(
        &self,
        pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        let pseudo = match self.try_stylo_element_data().and_then(|s| s.get()) {
            Some(el) => el
                .styles
                .get_primary()
                .and_then(|s| s.pseudo())
                .or(match &self.data {
                    NodeData::AnonymousBlock(_) => Some(PseudoElement::ServoAnonymousBox),
                    _ => None,
                }),
            None => None,
        };

        pseudo.is_some_and(|psuedo| psuedo == *pe)
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        // Handle flags that apply to the element.
        let self_flags = flags.for_self();
        if !self_flags.is_empty() {
            self.selector_flags()
                .set(self.selector_flags().get() | self_flags);
        }

        // Handle flags that apply to the parent.
        let parent_flags = flags.for_parent();
        if !parent_flags.is_empty() {
            if let Some(parent) = self.parent_node() {
                parent
                    .selector_flags()
                    .set(parent.selector_flags().get() | parent_flags);
            }
        }
    }

    fn is_link(&self) -> bool {
        self.data.is_element_with_tag_name(&local_name!("a"))
    }

    fn is_html_slot_element(&self) -> bool {
        self.element_data().is_some_and(|data|data.name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&data.name.local.as_ref()=="slot")
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        self.element_data()
            .and_then(|data| data.id.as_ref())
            .map(|id_attr| case_sensitivity.eq_atom(id_attr, id))
            .unwrap_or(false)
    }

    fn has_class(
        &self,
        search_name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        let class_attr = self.data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme);
                if case_sensitivity.eq_atom(&atom, search_name) {
                    return true;
                }
            }
        }

        false
    }

    fn imported_part(
        &self,
        name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::SelectorImpl>::Identifier> {
        let value=self.attr(LocalName::from("exportparts"))?;
        for mapping in value.split(',') {
            let mut parts=mapping.split(':');let inner=parts.next()?.trim();
            let outer=parts.next().map(str::trim).unwrap_or(inner);
            if parts.next().is_some()||inner.is_empty()||outer.is_empty()||inner.chars().any(char::is_whitespace)||outer.chars().any(char::is_whitespace){continue;}
            if outer==name.0.as_ref(){return Some(AtomIdent::from(inner));}
        }None
    }

    fn is_part(&self, name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> bool {
        self.attr(LocalName::from("part")).is_some_and(|parts|parts.split_ascii_whitespace().any(|part|part==name.0.as_ref()))
    }

    fn is_empty(&self) -> bool {
        !self.children.iter().any(|id|{let child=self.with(*id);child.is_element()||child.text_data().is_some_and(|text|!text.content.is_empty())})
    }

    fn is_root(&self) -> bool {
        self.parent_node().is_some_and(|parent|parent.as_document().is_some())
    }

    fn has_custom_state(
        &self,
        name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> bool {
        self.native_custom_states.contains(name)
    }

    fn add_element_unique_hashes(&self, filter: &mut selectors::bloom::BloomFilter) -> bool {
        each_relevant_element_hash(*self, |hash| filter.insert_hash(hash & BLOOM_HASH_MASK));
        true
    }
}

impl<'a> TElement for BlitzNode<'a> {
    fn inheritance_parent(&self) -> Option<Self> {
        // CSS inheritance follows the flat tree, whereas selector parent_element
        // deliberately stops at shadow roots. Assigned nodes inherit their slot.
        <Self as TNode>::traversal_parent(self)
    }

    fn implemented_pseudo_element(&self) -> Option<PseudoElement> {
        if !matches!(self.data, NodeData::AnonymousBlock(_)) { return None; }
        self.try_stylo_element_data().and_then(|data| data.get()).and_then(|data| data.styles.get_primary().and_then(|style| style.pseudo())).filter(|pseudo| matches!(pseudo, PseudoElement::PickerSelect))
    }

    type ConcreteNode = BlitzNode<'a>;

    type TraversalChildrenIterator = Traverser<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn implicit_scope_for_sheet_in_shadow_root(
        opaque_host: OpaqueElement,
        sheet_index: usize,
    ) -> Option<ImplicitScopeRoot> {
        crate::shadow_style::implicit_scope(opaque_host,sheet_index)
    }

    fn traversal_children(&self) -> style::dom::LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(Traverser {
            // dom: self.tree(),
            parent: self,
            child_index: 0,
        })
    }

    fn is_html_element(&self) -> bool {
        self.element_data().is_some_and(|data|data.name.ns.as_ref()=="http://www.w3.org/1999/xhtml")
    }

    // not implemented.....
    fn is_mathml_element(&self) -> bool {
        self.element_data().is_some_and(|data|data.name.ns.as_ref()=="http://www.w3.org/1998/Math/MathML")
    }

    // need to check the namespace
    fn is_svg_element(&self) -> bool {
        self.element_data().is_some_and(|data|data.name.ns.as_ref()=="http://www.w3.org/2000/svg")
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        if self.native_inline_style_blocked{return None;}
        self.element_data()
            .expect("Not an element")
            .style_attribute
            .as_ref()
            .map(|f| f.borrow_arc())
    }

    fn state(&self) -> ElementState {
        *self.element_state()
    }

    fn each_part<F>(&self, mut callback: F)
    where F: FnMut(&AtomIdent) {
        if let Some(parts) = self.attr(LocalName::from("part")) {
            for name in parts.split_ascii_whitespace() { callback(&AtomIdent::from(name)); }
        }
    }

    fn each_exported_part<F>(&self, name: &AtomIdent, mut callback: F)
    where F: FnMut(&AtomIdent) {
        let Some(value) = self.attr(LocalName::from("exportparts")) else { return; };
        for mapping in value.split(',') {
            let mut parts = mapping.split(':');
            let inner = parts.next().unwrap_or("").trim();
            let outer = parts.next().map(str::trim).unwrap_or(inner);
            if parts.next().is_some() || inner.is_empty() || outer.is_empty()
                || inner.chars().any(char::is_whitespace) || outer.chars().any(char::is_whitespace) { continue; }
            if inner == name.0.as_ref() { callback(&AtomIdent::from(outer)); }
        }
    }

    fn has_part_attr(&self) -> bool {
        self.attr(LocalName::from("part")).is_some()
    }

    fn exports_any_part(&self) -> bool {
        self.attr(LocalName::from("exportparts")).is_some_and(|value|!value.trim().is_empty())
    }

    fn id(&self) -> Option<&style::Atom> {
        self.element_data().and_then(|data| data.id.as_ref())
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&style::values::AtomIdent),
    {
        let class_attr = self.data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme); // interns the string
                callback(AtomIdent::cast(&atom));
            }
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&style::LocalName),
    {
        if let Some(attrs) = self.data.attrs() {
            for attr in attrs.iter() {
                callback(&GenericAtomIdent(attr.name.local.clone()));
            }
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        Node::has_dirty_descendants(self)
    }

    fn has_snapshot(&self) -> bool {
        Node::has_snapshot(self)
    }

    fn handled_snapshot(&self) -> bool {
        self.snapshot_handled().load(Ordering::SeqCst)
    }

    unsafe fn set_handled_snapshot(&self) {
        self.snapshot_handled().store(true, Ordering::SeqCst);
    }

    // Stylo calls this on elements it is already traversing (or has already
    // reached via invalidation), so the ancestor chain must not be re-marked:
    // ancestors visited earlier in the preorder traversal have had their bits
    // cleared, and re-flagging them leaves stale `dirty_descendants` bits that
    // break the early-out invariant of `Node::mark_ancestors_dirty` (a set bit
    // implies all ancestors are set).
    unsafe fn set_dirty_descendants(&self) {
        Node::set_dirty_descendants(self);
    }

    unsafe fn unset_dirty_descendants(&self) {
        Node::unset_dirty_descendants(self);
    }

    fn store_children_to_process(&self, _n: isize) {
        unimplemented!()
    }

    fn did_process_child(&self) -> isize {
        unimplemented!()
    }

    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        // SAFETY: stylo traversal has exclusive access to nodes
        unsafe { self.stylo_element_data().ensure_init() }
    }

    unsafe fn clear_data(&self) {
        // SAFETY: stylo traversal has exclusive access to nodes
        unsafe { self.stylo_element_data().clear() }
    }

    fn has_data(&self) -> bool {
        self.try_stylo_element_data().is_some_and(|s| s.has_data())
    }

    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.try_stylo_element_data().and_then(|s| s.get())
    }

    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        unsafe { self.stylo_element_data().unsafe_stylo_only_mut() }
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        true
    }
    fn css_animations_rendered(&self)->bool{self.native_css_animations_rendered}

    fn has_animations(&self, context: &SharedStyleContext) -> bool {
        self.has_css_animations(context, None) || self.has_css_transitions(context, None)
    }

    fn has_css_animations(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_animations(&key)
    }

    fn has_css_transitions(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_transitions(&key)
    }

    fn animation_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        let opaque = TNode::opaque(&TElement::as_node(self));
        context.animations.get_animation_declarations(
            &AnimationSetKey::new_for_non_pseudo(opaque),
            context.current_time_for_animations,
            self.guard(),
        )
    }

    fn transition_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        let opaque = TNode::opaque(&TElement::as_node(self));
        context.animations.get_transition_declarations(
            &AnimationSetKey::new_for_non_pseudo(opaque),
            context.current_time_for_animations,
            self.guard(),
        )
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.shadow_root.map(|root|self.with(root))
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.containing_shadow.map(|root|self.with(root))
    }

    fn get_attr(&self, attr: &style::LocalName, ns: &style::Namespace) -> Option<String> {
        self.data.attrs()?.iter().find(|entry|entry.name.local==attr.0&&entry.name.ns==ns.0).map(|entry|entry.value.clone())
    }

    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        self.native_language.as_deref().map(Into::into)
    }

    fn match_element_lang(&self,override_lang:Option<Option<style::selector_parser::AttrValue>>,ranges:&style::selector_parser::Lang)->bool {
        let language=override_lang.map(|value|value.map(|value|value.to_string())).unwrap_or_else(||self.native_language.clone()).unwrap_or_default();
        ranges.iter().any(|range|{
            if range.is_empty(){return language.is_empty();}if language.is_empty(){return false;}
            let language=language.split('-').collect::<Vec<_>>();let range=range.split('-').collect::<Vec<_>>();
            if range[0]!="*"&&!range[0].eq_ignore_ascii_case(language[0]){return false;}
            let mut position=1;for subtag in &range[1..]{if *subtag=="*"{continue;}
                while position<language.len()&&!subtag.eq_ignore_ascii_case(language[position]){if language[position].len()==1{return false;}position+=1;}
                if position==language.len(){return false;}position+=1;
            }true
        })
    }

    fn is_html_document_body_element(&self) -> bool {
        // Check node is a <body> element
        let is_body_element = self.data.is_element_with_tag_name(&local_name!("body"));

        // If it isn't then return early
        if !is_body_element {
            return false;
        }

        // If it is then check if it is a child of the root (<html>) element
        let root_node = TNode::owner_doc(self);
        let root_element = TDocument::as_node(&root_node)
            .first_element_child()
            .unwrap();
        root_element.children.contains(&self.id)
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: VisitedHandlingMode,
        hints: &mut V,
    ) where
        V: Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
        let Some(elem) = self.data.downcast_element() else {
            return;
        };

        let tag = &elem.name.local;

        let mut push_style = |decl: PropertyDeclaration| {
            hints.push(ApplicableDeclarationBlock::from_declarations(
                Arc::new(
                    self.guard()
                        .wrap(PropertyDeclarationBlock::with_one(decl, Importance::Normal)),
                ),
                CascadeLevel::new(CascadeOrigin::PresHints),
                LayerOrder::root(),
            ));
        };

        // URL resolution belongs to the native owner Document, including its
        // legacy query encoding. Keep authored attribute bytes for selectors.
        if let Some(url)=self.native_legacy_background_url.as_ref() {
            if let Ok(base)=url::Url::parse(url) {
                let mut value=String::from("url(");
                if cssparser::serialize_string(url,&mut value).is_ok() {
                    value.push(')');
                    let mut declarations=style::properties::SourcePropertyDeclaration::default();
                    let property=style::properties::PropertyId::parse_enabled_for_all_content("background-image").expect("native property");
                    if style::properties::parse_one_declaration_into(&mut declarations,property,&value,style::stylesheets::Origin::Author,&style::stylesheets::UrlExtraData::from(base),None,style_traits::ParsingMode::DEFAULT,QuirksMode::NoQuirks,style::stylesheets::CssRuleType::Style).is_ok(){
                        for declaration in declarations.drain().declarations{push_style(declaration);}
                    }
                }
            }
        }

        // SVG presentation attributes have zero specificity and yield to every
        // author declaration, including an explicit `clip-path:none`. Resolve
        // resource references against their native owner, not the style mirror.
        if elem.name.ns.as_ref()=="http://www.w3.org/2000/svg" {
            let base=self.native_svg_document_url.as_deref().and_then(|value|url::Url::parse(value).ok()).unwrap_or_else(||url::Url::parse("about:blank").expect("constant URL"));
            let extra=style::stylesheets::UrlExtraData::from(base);
            for attr in elem.attrs() {
                if !attr.name.ns.as_ref().is_empty(){continue}
                let name=attr.name.local.as_ref();
                if !matches!(name,"clip-path"|"clip-rule"|"fill"|"fill-opacity"|"fill-rule"|"stroke"|"stroke-width"|"stroke-opacity"|"stroke-linecap"|"stroke-linejoin"|"stroke-miterlimit"|"stroke-dasharray"|"stroke-dashoffset"|"opacity"|"display"|"visibility"|"overflow"|"pointer-events"|"transform"|"transform-origin"|"font-family"|"font-size"|"font-style"|"font-weight"|"font-stretch"|"letter-spacing"|"word-spacing"|"text-anchor"|"dominant-baseline"|"alignment-baseline"|"baseline-shift"|"writing-mode"|"direction"|"color"|"color-interpolation"|"color-interpolation-filters"|"shape-rendering"|"text-rendering"|"vector-effect"|"mask"|"filter"|"marker-start"|"marker-mid"|"marker-end"|"x"|"y"|"width"|"height"|"cx"|"cy"|"r"|"rx"|"ry"|"d"){continue}
                let Ok(property)=style::properties::PropertyId::parse_enabled_for_all_content(name)else{continue};
                let mut declarations=style::properties::SourcePropertyDeclaration::default();
                let value=if name=="transform"{self.native_svg_transform.as_deref().unwrap_or(attr.value.as_str())}else{attr.value.as_str()};
                if style::properties::parse_one_declaration_into(&mut declarations,property,value,style::stylesheets::Origin::Author,&extra,None,style_traits::ParsingMode::ALLOW_UNITLESS_LENGTH,QuirksMode::NoQuirks,style::stylesheets::CssRuleType::Style).is_ok(){
                    for declaration in declarations.drain().declarations{push_style(declaration);}
                }
            }
        }

        // HTML list ordinal hints participate at PresHints origin, so author
        // counter properties override them through the ordinary CSS cascade.
        if elem.name.ns.as_ref()=="http://www.w3.org/1999/xhtml" {
            use style::values::{CustomIdent,generics::counters::CounterPair,specified::{CounterReset,CounterSet,Integer}};
            let name=||CustomIdent(Atom::from("list-item"));
            if *tag==local_name!("ol") {
                let reversed=elem.attr(local_name!("reversed")).is_some();
                let start=elem.attr(local_name!("start")).and_then(|value|style::servo::attr::parse_integer(value.chars()).ok());
                if start.is_some()||reversed {
                    let value=start.map(|value|value.saturating_add(if reversed{1}else{-1})).unwrap_or(i32::MIN);
                    push_style(PropertyDeclaration::CounterReset(CounterReset::new(vec![CounterPair{name:name(),value:Integer::new(value),is_reversed:reversed}])));
                }
            }
            if *tag==local_name!("li") {
                if let Some(value)=elem.attr(local_name!("value")).and_then(|value|style::servo::attr::parse_integer(value.chars()).ok()) {
                    push_style(PropertyDeclaration::CounterSet(CounterSet::new(vec![CounterPair{name:name(),value:Integer::new(value),is_reversed:false}])));
                }
            }
        }

        fn parse_color_attr(value: &str) -> Option<(u8, u8, u8, f32)> {
            if !value.starts_with('#') {
                return None;
            }

            let value = &value[1..];
            if value.len() == 3 {
                let r = u8::from_str_radix(&value[0..1], 16).ok()?;
                let g = u8::from_str_radix(&value[1..2], 16).ok()?;
                let b = u8::from_str_radix(&value[2..3], 16).ok()?;
                return Some((r, g, b, 1.0));
            }

            if value.len() == 6 {
                let r = u8::from_str_radix(&value[0..2], 16).ok()?;
                let g = u8::from_str_radix(&value[2..4], 16).ok()?;
                let b = u8::from_str_radix(&value[4..6], 16).ok()?;
                return Some((r, g, b, 1.0));
            }

            None
        }

        /// The HTML "rules for parsing dimension values" -- Stylo's
        /// implementation of them -- packaged as a specified
        /// `<length-percentage>`. `ignoring_zero` selects the spec's separate
        /// "maps to the dimension property (ignoring zero)" mapping, where a
        /// zero value is dropped rather than honoured.
        ///
        /// https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-dimension-values
        fn parse_size_attr(
            value: &str,
            ignoring_zero: bool,
        ) -> Option<style::values::specified::LengthPercentage> {
            use style::servo::attr::{
                LengthOrPercentageOrAuto, parse_length, parse_nonzero_length,
            };
            use style::values::specified::{LengthPercentage, NoCalcLength};
            let parsed = if ignoring_zero {
                parse_nonzero_length(value)
            } else {
                parse_length(value)
            };
            match parsed {
                LengthOrPercentageOrAuto::Length(length) => Some(LengthPercentage::Length(
                    NoCalcLength::from_px(length.to_f32_px()),
                )),
                LengthOrPercentageOrAuto::Percentage(fraction) => Some(
                    LengthPercentage::Percentage(NoCalcPercentage::new(fraction)),
                ),
                LengthOrPercentageOrAuto::Auto => None,
            }
        }

        /// Parse the value of an SVG `width`/`height` presentation attribute.
        /// Unlike the legacy HTML dimension attributes, these accept any CSS
        /// <length-percentage> (e.g. `1em`), and a unitless number means user
        /// units, which map to CSS px.
        fn parse_svg_size_attr(value: &str) -> Option<style::values::specified::LengthPercentage> {
            use style::values::specified::{LengthPercentage, NoCalcLength};
            use style_traits::ParsingMode;

            let value = value.trim();
            if let Some(number) = value.strip_suffix('%') {
                let val: f32 = number.trim().parse().ok()?;
                return (val >= 0.0)
                    .then(|| LengthPercentage::Percentage(NoCalcPercentage::new(val / 100.0)));
            }

            // Split into number and unit: the unit is the trailing run of
            // ASCII alphabetic characters (this never eats into a scientific
            // exponent such as `1e3`, which ends in a digit).
            let number_len = value
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .len();
            let (number, unit) = value.split_at(number_len);
            let val: f32 = number.trim().parse().ok().filter(|v| *v >= 0.0)?;
            let length = if unit.is_empty() {
                NoCalcLength::from_px(val)
            } else {
                NoCalcLength::parse_dimension_with_flags(ParsingMode::DEFAULT, false, val, unit)
                    .ok()?
            };
            Some(LengthPercentage::Length(length))
        }

        // `<input type=image>` is the only input type that is replaced
        // content, and it is the only one that takes the embedded-content
        // presentational attributes. The type attribute is matched ASCII
        // case-insensitively, as attribute keywords always are.
        let is_image_input = *tag == local_name!("input")
            && elem.attrs().iter().any(|attr| {
                attr.name.local == local_name!("type") && attr.value.eq_ignore_ascii_case("image")
            });

        for attr in elem.attrs() {
            let name = &attr.name.local;
            let value = attr.value.as_str();

            if *name == local_name!("align") && *tag != local_name!("table") {
                use style::values::specified::TextAlign;
                let keyword = match value.to_ascii_lowercase().as_str() {
                    "left" => Some(StyloTextAlign::MozLeft),
                    "right" => Some(StyloTextAlign::MozRight),
                    "center" => Some(StyloTextAlign::MozCenter),
                    "justify" => Some(StyloTextAlign::Justify),
                    _ => None,
                };

                if let Some(keyword) = keyword {
                    push_style(PropertyDeclaration::TextAlign(TextAlign::Keyword(keyword)));
                }
            }

            // The width/height attributes on these elements map to the
            // corresponding dimension properties (percentages allowed):
            // https://html.spec.whatwg.org/multipage/rendering.html#dimRendering
            // https://html.spec.whatwg.org/multipage/rendering.html#attributes-for-embedded-content-and-images
            let is_width = *name == local_name!("width");
            let is_height = *name == local_name!("height");
            // The elements whose width/height attributes map to the dimension
            // properties. `input` only joins them as type=image, which is the
            // one replaced input type.
            let is_embedded = *tag == local_name!("iframe")
                || *tag == local_name!("embed")
                || *tag == local_name!("video")
                || *tag == local_name!("object")
                || *tag == local_name!("img")
                || *tag == local_name!("marquee")
                || is_image_input;
            let maps_to_dimension = if is_width {
                is_embedded
                    || *tag == local_name!("table")
                    || *tag == local_name!("col")
                    || *tag == local_name!("colgroup")
                    || *tag == local_name!("tr")
                    || *tag == local_name!("td")
                    || *tag == local_name!("th")
                    || *tag == local_name!("hr")
            } else if is_height {
                is_embedded
                    || *tag == local_name!("table")
                    || *tag == local_name!("thead")
                    || *tag == local_name!("tbody")
                    || *tag == local_name!("tfoot")
                    || *tag == local_name!("tr")
                    || *tag == local_name!("td")
                    || *tag == local_name!("th")
            } else {
                false
            };
            if maps_to_dimension {
                // Three of these use the "(ignoring zero)" variant of the
                // mapping, where a zero is dropped instead of honoured:
                // `table width`, and `td`/`th` in both axes.
                let is_cell = *tag == local_name!("td") || *tag == local_name!("th");
                let ignoring_zero = is_cell || (is_width && *tag == local_name!("table"));
                if let Some(size) = parse_size_attr(value, ignoring_zero) {
                    use style::values::generics::{NonNegative, length::Size};
                    let size = Size::LengthPercentage(NonNegative(size));
                    push_style(if is_width {
                        PropertyDeclaration::Width(size)
                    } else {
                        PropertyDeclaration::Height(size)
                    });
                }
            }

            // hspace/vspace map to the horizontal and vertical margins as
            // dimension properties. This is a *smaller* set than the one that
            // takes width/height: `iframe` and `video` take width and height
            // but not hspace/vspace, and html/rendering/unmapped-attributes
            // checks exactly that -- browsers have got it wrong before.
            let takes_spacing = *tag == local_name!("embed")
                || *tag == local_name!("img")
                || *tag == local_name!("object")
                || *tag == local_name!("marquee")
                || is_image_input;
            if takes_spacing {
                let is_hspace = *name == local_name!("hspace");
                let is_vspace = *name == local_name!("vspace");
                if is_hspace || is_vspace {
                    if let Some(size) = parse_size_attr(value, false) {
                        use style::values::generics::length::GenericMargin;
                        let margin = GenericMargin::LengthPercentage(size);
                        if is_hspace {
                            push_style(PropertyDeclaration::MarginLeft(margin.clone()));
                            push_style(PropertyDeclaration::MarginRight(margin));
                        } else {
                            push_style(PropertyDeclaration::MarginTop(margin.clone()));
                            push_style(PropertyDeclaration::MarginBottom(margin));
                        }
                    }
                }
            }

            // https://svgwg.org/svg2-draft/geometry.html#Sizing
            // The `width` and `height` attributes on an `<svg>` element are
            // presentation attributes that map to the CSS `width`/`height`
            // properties, so e.g. `width="1em"` must resolve against the
            // element's font-size like any other CSS length.
            if *tag == local_name!("svg")
                && (*name == local_name!("width") || *name == local_name!("height"))
            {
                if let Some(size) = parse_svg_size_attr(value) {
                    use style::values::generics::{NonNegative, length::Size};
                    let size = Size::LengthPercentage(NonNegative(size));
                    push_style(if *name == local_name!("width") {
                        PropertyDeclaration::Width(size)
                    } else {
                        PropertyDeclaration::Height(size)
                    });
                }
            }

            // The `border` attribute maps to the four border widths as a
            // pixel length, plus the four border styles as `solid` -- width
            // alone would compute back to zero against the default
            // border-style of `none`. It is only these three elements:
            // `embed`, `iframe`, `marquee` and non-image `input` all have a
            // `border` attribute that must stay unmapped.
            if *name == local_name!("border")
                && (*tag == local_name!("img") || *tag == local_name!("object") || is_image_input)
            {
                if let Ok(px) = style::servo::attr::parse_unsigned_integer(value.chars()) {
                    use style::values::specified::{BorderSideWidth, BorderStyle};
                    let width = BorderSideWidth::from_px(px as f32);
                    push_style(PropertyDeclaration::BorderTopWidth(width.clone()));
                    push_style(PropertyDeclaration::BorderRightWidth(width.clone()));
                    push_style(PropertyDeclaration::BorderBottomWidth(width.clone()));
                    push_style(PropertyDeclaration::BorderLeftWidth(width));
                    push_style(PropertyDeclaration::BorderTopStyle(BorderStyle::Solid));
                    push_style(PropertyDeclaration::BorderRightStyle(BorderStyle::Solid));
                    push_style(PropertyDeclaration::BorderBottomStyle(BorderStyle::Solid));
                    push_style(PropertyDeclaration::BorderLeftStyle(BorderStyle::Solid));
                }
            }

            // `body` carries four legacy margin attributes, as pixel lengths:
            // marginwidth and marginheight set both sides of an axis, and
            // leftmargin and topmargin set one side each.
            //
            // There is deliberately no `rightmargin` or `bottommargin`. They
            // look like the obvious counterparts to the two that exist, but
            // the spec does not define them and browsers ignore them in both
            // standards and quirks mode -- body-margin-3a/3b assert exactly
            // that.
            if *tag == local_name!("body") {
                // Matched as strings: these are not in the static atom set,
                // so `local_name!` will not compile for them.
                let sides: &[u8] = match &**name {
                    "marginwidth" => b"lr",
                    "marginheight" => b"tb",
                    "leftmargin" => b"l",
                    "topmargin" => b"t",
                    _ => b"",
                };
                if !sides.is_empty() {
                    if let Ok(px) = style::servo::attr::parse_unsigned_integer(value.chars()) {
                        use style::values::generics::length::GenericMargin;
                        use style::values::specified::{LengthPercentage, NoCalcLength};
                        let margin = GenericMargin::LengthPercentage(LengthPercentage::Length(
                            NoCalcLength::from_px(px as f32),
                        ));
                        for side in sides {
                            push_style(match side {
                                b'l' => PropertyDeclaration::MarginLeft(margin.clone()),
                                b'r' => PropertyDeclaration::MarginRight(margin.clone()),
                                b't' => PropertyDeclaration::MarginTop(margin.clone()),
                                b'b' => PropertyDeclaration::MarginBottom(margin.clone()),
                                _ => unreachable!("side table above only yields lrtb"),
                            });
                        }
                    }
                }
            }

            if *name == local_name!("bgcolor") {
                use style::values::specified::Color;
                if let Some((r, g, b, a)) = parse_color_attr(value) {
                    push_style(PropertyDeclaration::BackgroundColor(
                        Color::from_absolute_color(AbsoluteColor::srgb_legacy(r, g, b, a)),
                    ));
                }
            }

            if *name == local_name!("hidden") {
                use style::values::specified::Display;
                push_style(PropertyDeclaration::Display(Display::None));
            }
        }
    }

    fn local_name(&self) -> &LocalName {
        &self.element_data().expect("Not an element").name.local
    }

    fn namespace(&self) -> &Namespace {
        &self.element_data().expect("Not an element").name.ns
    }

    fn query_container_size(
        &self,
        _display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        // FIXME: Implement container queries. For now this effectively disables them without panicking.
        Default::default()
    }

    fn each_custom_state<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        for state in &self.native_custom_states{callback(state);}
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.selector_flags().get().contains(flags)
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        let flags = self.selector_flags().get();
        if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING) {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING
        } else {
            ElementSelectorFlags::empty()
        }
    }

    fn compute_layout_damage(old: &ComputedValues, new: &ComputedValues) -> RestyleDamage {
        compute_layout_damage(old, new)
        // ALL_DAMAGE
    }

    // fn update_animations(
    //     &self,
    //     before_change_style: Option<Arc<ComputedValues>>,
    //     tasks: style::context::UpdateAnimationsTasks,
    // ) {
    //     todo!()
    // }

    // fn process_post_animation(&self, tasks: style::context::PostAnimationTasks) {
    //     todo!()
    // }

    // fn needs_transitions_update(
    //     &self,
    //     before_change_style: &ComputedValues,
    //     after_change_style: &ComputedValues,
    // ) -> bool {
    //     todo!()
    // }
}

pub struct Traverser<'a> {
    // dom: &'a Slab<Node>,
    parent: BlitzNode<'a>,
    child_index: usize,
}

impl<'a> Iterator for Traverser<'a> {
    type Item = BlitzNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let children=self.parent.style_children.as_deref().unwrap_or(&self.parent.children);
        let node_id = children.get(self.child_index)?;
        let node = self.parent.with(*node_id);

        self.child_index += 1;

        Some(node)
    }
}

impl std::hash::Hash for BlitzNode<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.id.as_u64())
    }
}

/// Handle custom painters like images for layouting
///
/// todo: actually implement this
pub struct RegisteredPaintersImpl;
impl RegisteredSpeculativePainters for RegisteredPaintersImpl {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

use style::traversal::recalc_style_at;

pub struct RecalcStyle<'a> {
    cancelled: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    cancellation_observed: std::sync::atomic::AtomicBool,
    context: SharedStyleContext<'a>,
    /// Nodes whose `background-image`/`mask-image` layers need flushing to
    /// dedicated storage on the node (see `flush_image_layers_from_style`)
    /// because their style changed during this traversal.
    nodes_needing_style_image_flush: Mutex<Vec<NodeId>>,
}

impl<'a> RecalcStyle<'a> {
    pub fn new(context: SharedStyleContext<'a>) -> Self {
        RecalcStyle {
            cancelled: None,
            cancellation_observed: std::sync::atomic::AtomicBool::new(false),
            context,
            nodes_needing_style_image_flush: Mutex::new(Vec::new()),
        }
    }
}

#[allow(unsafe_code)]
impl<'dom> DomTraversal<BlitzNode<'dom>> for RecalcStyle<'_> {
    fn cancelled(&self) -> bool {
        use std::sync::atomic::Ordering;
        if self.cancellation_observed.load(Ordering::Acquire) { return true; }
        if self.cancelled.as_ref().is_some_and(|check| check()) {
            self.cancellation_observed.store(true, Ordering::Release);
            return true;
        }
        false
    }
    fn process_preorder<F: FnMut(BlitzNode<'dom>)>(
        &self,
        traversal_data: &PerLevelTraversalData,
        context: &mut StyleContext<BlitzNode<'dom>>,
        node: BlitzNode<'dom>,
        note_child: F,
    ) {
        if let Some(el) = node.as_element() {
            // let mut data = el.mutate_data().unwrap();
            let mut data = unsafe { el.ensure_data() };
            recalc_style_at(self, traversal_data, context, el, &mut data, note_child);

            sync_pseudo_element_styles(el, &data, &self.nodes_needing_style_image_flush);

            if !data.damage.is_empty() {
                // Mark the ancestor chain so that the damage propagation pass
                // visits this element.
                el.mark_damaged();

                if data
                    .styles
                    .get_primary()
                    .is_some_and(|style| needs_style_image_flush(el, style))
                {
                    self.nodes_needing_style_image_flush
                        .lock()
                        .unwrap()
                        .push(el.id);
                }
            }

            // Gets set later on
            el.unset_dirty_descendants();
        }
    }

    #[inline]
    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(
        &self,
        _style_context: &mut StyleContext<BlitzNode<'dom>>,
        _node: BlitzNode<'dom>,
    ) {
        panic!("this should never be called")
    }

    #[inline]
    fn shared_context(&self) -> &SharedStyleContext<'_> {
        &self.context
    }
}

/// Flush updated pseudo-element (`::before`/`::after`) styles from the owning
/// element's freshly-computed stylo data to the pseudo-element's anonymous node.
///
/// Pseudo-element styles are normally flushed to the pseudo-element's node
/// during box construction (see `flush_pseudo_elements`), but in incremental
/// mode box construction only runs for nodes with construction damage.
/// Pseudo-element style changes which don't require reconstruction (e.g.
/// animations/transitions of repaint- or relayout-only properties) must still
/// be flushed to the pseudo-element's node - along with the damage they imply -
/// so that layout and paint see the new style.
///
/// This runs during the style traversal, immediately after the owning element
/// has been restyled, because that is where the old and new pseudo styles can
/// be diffed precisely (the pseudo node's stored primary style is the "old"
/// style; the owner's just-computed pseudo styles are the "new" ones).
///
/// SAFETY: the style traversal has exclusive access to the tree, and each
/// pseudo-element node is only ever accessed from its owning element's
/// `process_preorder` (pseudo nodes are not part of the DOM child list and are
/// not themselves visited by the traversal), so mutating the pseudo node's
/// stylo data here cannot race with any other traversal thread.
#[allow(unsafe_code)]
fn sync_pseudo_element_styles(
    el: &Node,
    data: &style::data::ElementData,
    nodes_needing_style_image_flush: &Mutex<Vec<NodeId>>,
) {
    let before_node_id = el.before();
    let after_node_id = el.after();
    if before_node_id.is_none() && after_node_id.is_none() {
        return;
    }

    // Note: yes these are kinda backwards (see `flush_pseudo_elements`)
    let pseudos = data.styles.pseudos.as_array();
    let before_style = pseudos[1].clone();
    let after_style = pseudos[0].clone();

    // Creation and removal of pseudo-elements is handled during box construction
    // (Stylo generates construction damage for those cases), so only the case
    // where the pseudo-element both was and remains present is handled here.
    for (pe_node_id, pe_style) in [(before_node_id, before_style), (after_node_id, after_style)] {
        let (Some(pe_node_id), Some(pe_style)) = (pe_node_id, pe_style) else {
            continue;
        };
        let pe_node = el.with(pe_node_id);
        let Some(stylo_data) = pe_node.try_stylo_element_data() else {
            continue;
        };
        let mut pe_data = match unsafe { stylo_data.unsafe_stylo_only_mut() } {
            Some(data) => data,
            None => continue,
        };
        let Some(old_style) = pe_data.styles.primary.clone() else {
            continue;
        };
        if std::ptr::eq(&*old_style, &*pe_style) {
            continue;
        }

        let diff = RestyleDamage::compute_style_difference::<&Node>(&old_style, &pe_style);
        if !diff.damage.is_empty() {
            pe_data.damage.insert(diff.damage);
            pe_node.mark_damaged();

            if needs_style_image_flush(pe_node, &pe_style) {
                nodes_needing_style_image_flush
                    .lock()
                    .unwrap()
                    .push(pe_node_id);
            }
        }
        pe_data.styles.primary = Some(pe_style);
        pe_data.set_restyled();
    }
}

/// Whether a node's `background-image`/`mask-image` layers need flushing to
/// dedicated storage on the node: the URLs referenced by the new style differ
/// from the image data currently stored on the node.
fn needs_style_image_flush(node: &Node, style: &ComputedValues) -> bool {
    use crate::node::ImageResourceData;
    use style::url::ComputedUrl;
    use style::values::computed::image::Image;

    fn out_of_sync(style_images: &[Image], stored: &[Option<ImageResourceData>]) -> bool {
        if style_images.len() != stored.len() {
            // Differing lengths only matter if either side references an image
            // (a style list of `none` layers and empty storage are in sync).
            return style_images
                .iter()
                .any(|image| matches!(image, Image::Url(_)))
                || stored.iter().any(Option::is_some);
        }
        std::iter::zip(style_images, stored).any(|(style_image, stored)| {
            match (style_image, stored) {
                (Image::Url(ComputedUrl::Valid(url)), Some(data)) => **url != *data.url,
                (Image::Url(ComputedUrl::Valid(_)), None) => true,
                (_, Some(_)) => true,
                (_, None) => false,
            }
        })
    }

    node.data.downcast_element().is_some_and(|el| {
        out_of_sync(
            &style.get_background().background_image.0,
            &el.background_images,
        ) || out_of_sync(&style.get_svg().mask_image.0, &el.mask_images)
    })
}

#[test]
fn assert_size_of_equals() {
    // use std::mem;

    // fn assert_layout<E>() {
    //     assert_eq!(
    //         mem::size_of::<SharingCache<E>>(),
    //         mem::size_of::<TypelessSharingCache>()
    //     );
    //     assert_eq!(
    //         mem::align_of::<SharingCache<E>>(),
    //         mem::align_of::<TypelessSharingCache>()
    //     );
    // }

    // let size = mem::size_of::<StyleSharingCandidate<BlitzNode>>();
    // dbg!(size);
}

#[test]
fn parse_inline() {
    // let attrs = style::attr::AttrValue::from_serialized_tokenlist(
    //     r#"visibility: hidden; left: 1306.5px; top: 50px; display: none;"#.to_string(),
    // );

    // let val = CSSInlineStyleDeclaration();
}
