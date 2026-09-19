//! Native Web Animations sampling. Property parsing, computed-value resolution,
//! easing, interpolation and additive composition all use Stylo's value model.
use std::collections::BTreeMap;
use style_traits::ToCss;
use blitz_traits::node_id::NodeId;
use style::{animation::AnimationSetKey,context::{CascadeInputs,QuirksMode},dom::{TNode,TElement,OpaqueNode},properties::{ComputedValues,PropertyDeclarationBlock},properties::animated_properties::{AnimationValue,AnimationValueMap},rule_tree::{CascadeLevel,CascadeOrigin},servo_arc::Arc,shared_lock::StylesheetGuards,stylesheets::{CssRuleType,layer_rule::LayerOrder},values::animated::{Animate,Procedure}};
use style::selector_parser::PseudoElement;
#[derive(Clone,Debug,Default,PartialEq)]
pub struct NativeAnimationKeyframe {pub offset:f64,pub easing:String,pub composite:String,pub properties:BTreeMap<String,String>}
#[derive(Clone,Debug,Default,PartialEq)]
pub struct NativeAnimationEffect {pub id:u64,pub target:u64,pub pseudo:Option<PseudoElement>,pub keyframes:Vec<NativeAnimationKeyframe>,pub progress:Option<f64>,pub current_iteration:f64,pub composite:String}
impl crate::BaseDocument {
    pub(crate) fn animation_cascade(&self,id:NodeId,base:&Arc<ComputedValues>,block:PropertyDeclarationBlock)->Arc<ComputedValues>{
        let Some(element)=self.get_node(id).and_then(|node|node.as_element())else{return base.clone()};
        let block=Arc::new(self.guard.wrap(block));let guard=self.guard.read();let guards=StylesheetGuards::same(&guard);
        let rules=self.stylist.rule_tree().update_rule_at_level(CascadeLevel::new(CascadeOrigin::Animations),LayerOrder::root(),Some(block.borrow_arc()),base.rules(),&guards,&mut false);
        let Some(rules)=rules else{return base.clone()};
        let mut inputs=CascadeInputs::new_from_style(base);inputs.rules=Some(rules);
        let parent=if base.pseudo().is_some(){element.primary_styles().map(|style|Arc::clone(&style))}else{TNode::traversal_parent(&element).and_then(|parent|parent.as_element()).and_then(|parent|parent.primary_styles().map(|style|Arc::clone(&style)))};
        self.stylist.cascade_style_and_visited(Some(element),base.pseudo().as_ref(),&inputs,&guards,parent.as_deref(),parent.as_deref(),style::properties::FirstLineReparenting::No,&Default::default(),None,&mut Default::default(),&mut Default::default())
    }
    fn animation_block(&self,properties:&BTreeMap<String,String>)->PropertyDeclarationBlock{
        // Parse each declaration separately so authored values cannot introduce
        // additional declarations when serialized into the private effect stack.
        let mut result=PropertyDeclarationBlock::new();
        for(name,value)in properties {
            let Ok(property)=style::properties::PropertyId::parse_enabled_for_all_content(name)else{continue};
            let mut declarations=style::properties::SourcePropertyDeclaration::default();
            if style::properties::parse_one_declaration_into(&mut declarations,property,value,style::stylesheets::Origin::Author,&self.url.url_extra_data(),None,style_traits::ParsingMode::DEFAULT,QuirksMode::NoQuirks,CssRuleType::Keyframe).is_ok(){result.extend(declarations.drain(),style::properties::Importance::Normal);}

        }result
    }
    pub fn sample_native_effects(&mut self,ids:&BTreeMap<u64,NodeId>,effects:&[NativeAnimationEffect],now:f64)->BTreeMap<u64,BTreeMap<String,String>>{
        struct LayoutThread(bool);impl Drop for LayoutThread{fn drop(&mut self){if self.0{style::thread_state::exit(style::thread_state::ThreadState::LAYOUT);}}}
        let enter=!style::thread_state::get().contains(style::thread_state::ThreadState::LAYOUT);if enter{style::thread_state::enter(style::thread_state::ThreadState::LAYOUT);}let _thread=LayoutThread(enter);
        for set in self.animations.sets.write().values_mut(){set.external_values=None;}
        let mut samples=BTreeMap::new();
        for effect in effects {
            let Some(progress)=effect.progress else{continue};let Some(id)=ids.get(&effect.target).copied()else{continue};
            let Some(base)=(match effect.pseudo.as_ref(){Some(pseudo)=>self.resolved_pseudo_style(id,pseudo),None=>self.resolved_primary_style(id)})else{continue};
            let key=AnimationSetKey::new(OpaqueNode(id.as_u64() as usize),effect.pseudo.clone());
            let mut underlying=self.animations.sets.read().get(&key).and_then(|set|set.get_value_map_for_active_animations(now)).unwrap_or_default();
            let mut properties:BTreeMap<String,Vec<(f64,AnimationValue,String,String)>>=BTreeMap::new();
            for frame in &effect.keyframes {
                let block=self.animation_block(&frame.properties);let declared=block.declarations().iter().map(|declaration|declaration.id().to_owned()).collect::<Vec<_>>();
                let computed=self.animation_cascade(id,&base,block);
                for property in declared {let physical=property.as_borrowed().to_physical(base.writing_mode);let Some(value)=AnimationValue::from_computed_values(physical,&computed)else{continue};properties.entry(value.id().name().to_string()).or_default().push((frame.offset,value,frame.easing.clone(),frame.composite.clone()));}
            }
            let mut sampled=AnimationValueMap::default();
            for (_name,mut frames) in properties {
                frames.sort_by(|a,b|a.0.total_cmp(&b.0));let property=frames[0].1.id().to_owned();
                let Some(original)=underlying.get(&property).cloned().or_else(||AnimationValue::from_computed_values(property.as_borrowed(),&base))else{continue};
                if frames[0].0>0.{frames.insert(0,(0.,original.clone(),"linear".into(),"replace".into()));}
                if frames.last().is_some_and(|frame|frame.0<1.){frames.push((1.,original.clone(),"linear".into(),"replace".into()));}
                if frames.len()==1{frames.push(frames[0].clone());}
                let right=frames.iter().position(|frame|frame.0>progress).unwrap_or(frames.len()-1).max(1);let left=right-1;
                let (a,b)=(&frames[left],&frames[right]);let local=if b.0==a.0{1.}else{(progress-a.0)/(b.0-a.0)};
                let timing=self.animation_cascade(id,&base,self.animation_block(&BTreeMap::from([("animation-timing-function".into(),a.2.clone())])));
                let local=timing.get_ui().animation_timing_function_mod(0).calculate_output(local,style::values::generics::easing::BeforeFlag::Unset,1e-7);
                let compose=|value:&AnimationValue,mode:&str|{let mode=if mode=="auto"||mode.is_empty(){effect.composite.as_str()}else{mode};match mode{"add"=>original.animate(value,Procedure::Add).unwrap_or_else(|_|value.clone()),"accumulate"=>original.animate(value,Procedure::Accumulate{count:1}).unwrap_or_else(|_|value.clone()),_=>value.clone()}};
                let from=compose(&a.1,&a.3);let to=compose(&b.1,&b.3);
                let value=from.animate(&to,Procedure::Interpolate{progress:local}).unwrap_or_else(|_|if local<0.5{from}else{to});sampled.insert(property,value);
            }
            underlying.extend(sampled.clone());
            let computed=self.animation_cascade(id,&base,PropertyDeclarationBlock::from_animation_value_map(&sampled));
            let mut serialized=BTreeMap::new();for value in sampled.values(){serialized.insert(value.id().name().to_string(),computed.computed_value_to_string(value.id()));}
            samples.insert(effect.id,serialized);
            let mut sets=self.animations.sets.write();let set=sets.entry(key).or_default();set.external_values=Some(underlying);set.dirty=true;
        }
        for id in ids.values(){if let Some(node)=self.nodes.get_mut(*id){node.set_restyle_hint(style::invalidation::element::restyle_hints::RestyleHint::RESTYLE_SELF);}}
        samples
    }
}

#[derive(Clone,Debug)]
pub struct NativeCssAnimation {pub timing_direction:String,pub timing_fill:String,pub timing_easing:String,pub in_effect:bool,pub transition:bool,pub delay_ms:f64,pub iteration_duration_ms:f64,pub iterations:f64,pub id:u64,pub target:u64,pub pseudo:Option<PseudoElement>,pub name:String,pub current_time_ms:f64,pub duration_ms:f64,pub play_state:String}
#[derive(Clone,Copy,Debug)]
pub enum NativeCssAnimationControl{Pause,Play,Finish,Cancel,SetCurrentTime}
impl crate::StyleContinuity {
    pub fn next_animation_id(&self)->u64{self.next_id}
    pub fn assign_animation_ids(&mut self,next:u64){
        self.next_id=next.max(1);
        for (_,_,set) in &mut self.sets {
            for animation in &mut set.animations{if animation.native_id==0{animation.native_id=self.next_id;self.next_id=self.next_id.saturating_add(1);}}
            for transition in &mut set.transitions{if transition.native_id==0{transition.native_id=self.next_id;self.next_id=self.next_id.saturating_add(1);}}
        }
    }
    pub fn css_animations(&self,now:f64)->Vec<NativeCssAnimation>{
        use style::animation::{AnimationState,KeyframesIterationState};
        let state=|state:&AnimationState|match state{AnimationState::Paused(_)=>"paused",AnimationState::Finished=>"finished",AnimationState::Canceled=>"idle",_=>"running"}.to_owned();
        let mut result=Vec::new();
        for(target,pseudo,set)in &self.sets {
            for animation in &set.animations {
                if animation.state==AnimationState::Canceled{continue;}
                let iterations=match animation.iteration_state{KeyframesIterationState::Finite(_,max)=>max,_=>f64::INFINITY};
                let completed=match animation.iteration_state{KeyframesIterationState::Finite(current,_)|KeyframesIterationState::Infinite(current)=>current};
                let time=if animation.state==AnimationState::Finished{(animation.delay+if animation.duration==0.{0.}else{animation.duration*iterations}).max(0.)}else{(match animation.state{AnimationState::Paused(progress)=>progress*animation.duration+animation.delay,_=>now-animation.started_at+animation.delay})+completed*animation.duration};
                result.push(NativeCssAnimation{timing_direction:animation.direction.to_css_string(),timing_fill:animation.fill_mode.to_css_string(),timing_easing:"linear".into(),in_effect:if time<animation.delay{matches!(animation.fill_mode,style::properties::longhands::animation_fill_mode::computed_value::single_value::T::Backwards|style::properties::longhands::animation_fill_mode::computed_value::single_value::T::Both)}else if animation.state==AnimationState::Finished{matches!(animation.fill_mode,style::properties::longhands::animation_fill_mode::computed_value::single_value::T::Forwards|style::properties::longhands::animation_fill_mode::computed_value::single_value::T::Both)}else{true},transition:false,delay_ms:animation.delay*1000.,iteration_duration_ms:animation.duration*1000.,iterations,id:animation.native_id,target:*target,pseudo:pseudo.clone(),name:animation.name.to_string(),current_time_ms:time*1000.,duration_ms:(animation.delay+if animation.duration==0.{0.}else{animation.duration*iterations}).max(0.)*1000.,play_state:state(&animation.state)});
            }
            for transition in &set.transitions {
                if transition.state==AnimationState::Canceled{continue;}
                let time=match transition.state{AnimationState::Paused(progress)=>progress*transition.property_animation.duration+transition.delay,_=>now-transition.start_time+transition.delay};
                let name=transition.calculate_value(now).id().name().to_string();
                result.push(NativeCssAnimation{timing_direction:"normal".into(),timing_fill:"backwards".into(),timing_easing:transition.native_timing_easing(),in_effect:transition.state!=AnimationState::Finished,transition:true,delay_ms:transition.delay*1000.,iteration_duration_ms:transition.property_animation.duration*1000.,iterations:1.,id:transition.native_id,target:*target,pseudo:pseudo.clone(),name,current_time_ms:time*1000.,duration_ms:(transition.delay+transition.property_animation.duration).max(0.)*1000.,play_state:state(&transition.state)});
            }
        }result.sort_by_key(|animation|animation.id);result
    }
    pub fn control_css_animation(&mut self,id:u64,control:NativeCssAnimationControl,time:Option<f64>,now:f64)->Result<(),String>{
        use style::animation::{AnimationState,KeyframesIterationState};
        for(_,_,set)in &mut self.sets {
            if let Some(animation)=set.animations.iter_mut().find(|animation|animation.native_id==id){
                let progress=match animation.state{AnimationState::Paused(progress)=>progress,_=>if animation.duration==0.{if now<animation.started_at{-1.}else{1.}}else{(now-animation.started_at)/animation.duration}};
                match control {
                    NativeCssAnimationControl::Pause=>animation.state=AnimationState::Paused(progress),
                    NativeCssAnimationControl::Play=>{if animation.state==AnimationState::Finished{animation.set_native_active_time(0.,now,false);}else{animation.started_at=now-progress*animation.duration;animation.state=AnimationState::Running;}},
                    NativeCssAnimationControl::Cancel=>animation.state=AnimationState::Canceled,
                    NativeCssAnimationControl::Finish=>{let iterations=match animation.iteration_state{KeyframesIterationState::Finite(_,max)=>max,_=>if animation.duration==0.{0.}else{return Err("InvalidStateError".into())}};animation.set_native_active_time(animation.duration*iterations,now,false);animation.state=AnimationState::Finished;},
                    NativeCssAnimationControl::SetCurrentTime=>{let time=time.ok_or("TypeError")?/1000.-animation.delay;animation.set_native_active_time(time,now,matches!(animation.state,AnimationState::Paused(_)));}
                }
                animation.native_controlled=true;set.dirty=true;self.active=true;return Ok(());
            }
            if let Some(transition)=set.transitions.iter_mut().find(|transition|transition.native_id==id){
                let duration=transition.property_animation.duration;let progress=match transition.state{AnimationState::Paused(progress)=>progress,_=>(now-transition.start_time)/duration};
                match control{NativeCssAnimationControl::Pause=>transition.state=AnimationState::Paused(progress),NativeCssAnimationControl::Play=>{transition.start_time=now-progress*duration;transition.state=AnimationState::Running;},NativeCssAnimationControl::Finish=>{transition.start_time=now-duration;transition.state=AnimationState::Finished;},NativeCssAnimationControl::Cancel=>transition.state=AnimationState::Canceled,NativeCssAnimationControl::SetCurrentTime=>{let time=time.ok_or("TypeError")?/1000.-transition.delay;if matches!(transition.state,AnimationState::Paused(_)){transition.state=AnimationState::Paused(time/duration);}else{transition.start_time=now-time;}}}
                set.dirty=true;self.active=true;return Ok(());
            }
        }Err("native CSS animation no longer exists".into())
    }
    pub fn css_animation_style_values(&self,id:u64,now:f64)->BTreeMap<String,String>{
        let mut values=AnimationValueMap::default();
        for(_,_,set)in &self.sets{for animation in &set.animations{if animation.native_id==id{animation.get_property_declaration_at_time(now,&mut values);}}for transition in &set.transitions{if transition.native_id==id{let value=transition.calculate_value(now);values.insert(value.id().to_owned(),value);}}}
        let mut result=BTreeMap::new();for value in values.values(){let mut serialized=style_traits::CssString::new();value.uncompute().to_css(&mut serialized).ok();result.insert(value.id().name().to_string(),serialized.into());}result
    }
}
