//! Generated source regressions for shared query/render animation authority.
use super::*;
fn article_width(snapshot: &oryn_common::v4::PaintSnapshot) -> f64 {
    snapshot
        .nodes
        .iter()
        .find(|node| node.tag == "article")
        .unwrap()
        .geometry
        .box_model
        .as_ref()
        .unwrap()
        .border_box
        .width
}
#[tokio::test(flavor = "current_thread")]
async fn paused_effect_time_and_keyframe_changes_invalidate_main_render_without_dom_mutation() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/paused",
        "<!doctype html><article style='height:10px'></article>",
    )
    .await
    .unwrap();
    page.evaluate("globalThis.effect=document.querySelector('article').animate([{width:'10px'},{width:'110px'}],{duration:1000,fill:'both'});effect.pause();effect.currentTime=500;getComputedStyle(document.querySelector('article')).width").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 60.);
    page.evaluate(
        "effect.currentTime=750;getComputedStyle(document.querySelector('article')).width",
    )
    .await
    .unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 85.);
    page.evaluate("effect.effect.setKeyframes([{width:'110px'},{width:'210px'}]);getComputedStyle(document.querySelector('article')).width").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 185.);
}
#[tokio::test(flavor = "current_thread")]
async fn terminal_css_geometry_query_is_published_to_main_paint_after_animation_stops() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/terminal","<!doctype html><style>@keyframes grow{from{width:10px}to{width:110px}}article{height:10px;animation:grow 1000s linear both}</style><article></article>").await.unwrap();
    let _ = page.paint_snapshot().await.unwrap();
    assert_eq!(page.evaluate("(()=>{const e=document.querySelector('article');e.getAnimations()[0].finish();return getComputedStyle(e).width})()").await.unwrap(),"110px");
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 110.);
}
#[tokio::test(flavor = "current_thread")]
async fn keyframe_constructor_and_set_keyframes_resolve_document_base_urls() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/page",
        "<!doctype html><base href='/non-existent-base/'><article></article>",
    )
    .await
    .unwrap();
    assert_eq!(page.evaluate(r#"(()=>{const target=document.querySelector('article');const frames=[{backgroundImage:'url(something.png)'},{backgroundImage:'url(something.png)'}];for(const initialize of [true,false]){const effect=new KeyframeEffect(target,initialize?frames:[],10000);if(!initialize)effect.setKeyframes(frames);const animation=new Animation(effect,document.timeline);animation.pause();animation.currentTime=5000;if(!getComputedStyle(target).backgroundImage.includes('/non-existent-base/something.png'))return false;animation.cancel();if(getComputedStyle(target).backgroundImage!=='none')return false;}return true})()"#).await.unwrap(),"true");
}
#[tokio::test(flavor = "current_thread")]
async fn commit_styles_publishes_one_custom_element_attribute_reaction() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/reactions",
        "<!doctype html><body></body>",
    )
    .await
    .unwrap();
    assert_eq!(page.evaluate(r#"(()=>{let changes=0;customElements.define('animated-element',class extends HTMLElement{static get observedAttributes(){return['style']}attributeChangedCallback(){changes++}});const target=document.createElement('animated-element');document.body.appendChild(target);const animation=target.animate([{opacity:0,width:'10px'},{opacity:1,width:'110px'}],{duration:100,fill:'both'});animation.pause();animation.currentTime=50;animation.commitStyles();return changes===1&&target.style.opacity==='0.5'&&target.style.width==='60px'})()"#).await.unwrap(),"true");
}
#[tokio::test(flavor = "current_thread")]
async fn paused_effect_edits_publish_without_an_intervening_style_query() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/dirty",
        "<!doctype html><article style='height:10px'></article>",
    )
    .await
    .unwrap();
    page.evaluate("globalThis.a=document.querySelector('article').animate([{width:'10px'},{width:'110px'}],{duration:100,fill:'both'});a.pause();a.currentTime=50;undefined").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 60.);
    page.evaluate("a.currentTime=75;undefined").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 85.);
    page.evaluate("a.effect.setKeyframes([{width:'110px'},{width:'210px'}]);undefined")
        .await
        .unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 185.);
}
#[tokio::test(flavor = "current_thread")]
async fn state_preserving_moves_keep_transition_identity_across_shadow_and_reject_other_documents()
{
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/move","<!doctype html><style>article{display:block;width:0;transition:width 10s linear}</style><article></article><div id='host'></div>").await.unwrap();
    assert_eq!(page.evaluate(r#"(()=>{const target=document.querySelector('article');getComputedStyle(target).width;target.style.width='1000px';getComputedStyle(target).width;const transition=target.getAnimations()[0];transition.pause();transition.currentTime=1000;const shadow=document.getElementById('host').attachShadow({mode:'open'});shadow.innerHTML='<style>article{display:block;transition:width 10s linear}</style><section></section>';shadow.querySelector('section').moveBefore(target,null);if(target.getAnimations()[0]!==transition)throw Error('move replaced transition identity: '+JSON.stringify({phase:'after-shadow-move',connected:target.isConnected,owner:target.ownerDocument===document,oldState:transition.playState,oldTime:transition.currentTime,animations:target.getAnimations().map(a=>({same:a===transition,type:a.constructor.name,property:a.transitionProperty,state:a.playState,time:a.currentTime})),width:getComputedStyle(target).width,property:getComputedStyle(target).transitionProperty,duration:getComputedStyle(target).transitionDuration}));if(getComputedStyle(target).width!=='100px')throw Error('moved sample: '+getComputedStyle(target).width);const other=document.implementation.createHTMLDocument();try{other.body.moveBefore(target,null);return false}catch(e){if(e.name!=='HierarchyRequestError')throw e}return target.getAnimations()[0]===transition&&getComputedStyle(target).width==='100px'})()"#).await.unwrap(),"true");
}
#[tokio::test(flavor = "current_thread")]
async fn move_before_starts_a_transition_for_changed_selector_values() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/move-selector","<!doctype html><style>article{display:block;width:0;transition:width 10s steps(1,jump-both)}#new article{width:400px}</style><section><article></article></section><section id='new'></section>").await.unwrap();
    assert_eq!(page.evaluate("(()=>{const target=document.querySelector('article');getComputedStyle(target).width;document.getElementById('new').moveBefore(target,null);return getComputedStyle(target).width})()").await.unwrap(),"200px");
}
#[tokio::test(flavor = "current_thread")]
async fn paused_css_keyframes_refresh_when_layered_stylesheets_change() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/layers","<!doctype html><style>@layer base,override;@layer base{@keyframes colorize{from{color:red}}}@layer override{@keyframes colorize{from{color:green}}}article{animation:colorize 1s paused}</style><article>text</article>").await.unwrap();
    assert_eq!(
        page.evaluate("getComputedStyle(document.querySelector('article')).color")
            .await
            .unwrap(),
        "rgb(0, 128, 0)"
    );
    assert_eq!(page.evaluate("globalThis.cssAnimation=document.querySelector('article').getAnimations()[0];const sheet=document.createElement('style');sheet.textContent='@keyframes colorize{from{color:blue}}';document.head.appendChild(sheet);getComputedStyle(document.querySelector('article')).color").await.unwrap(),"rgb(0, 0, 255)");
    assert_eq!(page.evaluate("document.querySelector('article').getAnimations()[0]===cssAnimation&&cssAnimation.playState==='paused'").await.unwrap(),"true");
}
#[tokio::test(flavor = "current_thread")]
async fn animation_revert_rolls_back_author_origin_while_revert_layer_retains_it() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/revert","<!doctype html><style>@keyframes first{from{width:revert}to{width:200px}}@keyframes second{from{width:revert-layer}to{width:200px}}article{width:100px;animation:first 2s linear -1s paused}article.second{animation-name:second}</style><article></article><article class='second'></article>").await.unwrap();
    assert_eq!(page.evaluate("Array.from(document.querySelectorAll('article'),e=>getComputedStyle(e).width).join(',')").await.unwrap(),"200px,150px");
}
#[tokio::test(flavor = "current_thread")]
async fn multiple_css_animations_update_without_skipping_later_names() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/multiple","<!doctype html><style>@keyframes wide{from{width:20px}}@keyframes tall{from{height:30px}}article{animation:wide 1s paused,tall 1s paused}</style><article></article>").await.unwrap();
    assert_eq!(page.evaluate("const e=document.querySelector('article');[getComputedStyle(e).width,getComputedStyle(e).height].join(',')").await.unwrap(),"20px,30px");
    assert_eq!(page.evaluate("const s=document.createElement('style');s.textContent='@keyframes wide{from{width:40px}}@keyframes tall{from{height:50px}}';document.head.appendChild(s);[getComputedStyle(e).width,getComputedStyle(e).height].join(',')").await.unwrap(),"40px,50px");
}
#[tokio::test(flavor = "current_thread")]
async fn paused_zero_duration_css_delay_changes_keep_finite_samples() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://animation.test/zero-delay","<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}article{animation:fade 0s 1s both paused}</style><article></article>").await.unwrap();
    assert_eq!(
        page.evaluate("const e=document.querySelector('article');getComputedStyle(e).opacity")
            .await
            .unwrap(),
        "0"
    );
    assert_eq!(
        page.evaluate("e.style.animationDelay='-1s';getComputedStyle(e).opacity")
            .await
            .unwrap(),
        "1"
    );
    assert_eq!(
        page.evaluate("e.style.animationDelay='2s';getComputedStyle(e).opacity")
            .await
            .unwrap(),
        "0"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn script_keyframe_replacement_distinguishes_origin_and_layer_rollback() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/set-keyframes-revert",
        "<!doctype html><article style='width:100px'></article>",
    )
    .await
    .unwrap();
    assert_eq!(page.evaluate("const e=document.querySelector('article');const a=e.animate([{width:'400px'},{width:'800px'}],{duration:1000000,delay:-500000,easing:'steps(2,end)'});getComputedStyle(e).width").await.unwrap(),"600px");
    assert_eq!(
        page.evaluate(
            "a.effect.setKeyframes([{width:'revert'},{width:'300px'}]);getComputedStyle(e).width"
        )
        .await
        .unwrap(),
        "300px"
    );
    assert_eq!(page.evaluate("e.style.width='200px';a.effect.setKeyframes([{width:'revert-layer'},{width:'400px'}]);getComputedStyle(e).width").await.unwrap(),"300px");
}
#[tokio::test(flavor = "current_thread")]
async fn script_effect_stack_composes_each_effect_over_the_previous_sample() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/effect-stack",
        "<!doctype html><article style='width:100px'></article>",
    )
    .await
    .unwrap();
    assert_eq!(page.evaluate("const e=document.querySelector('article');const a=e.animate([{width:'10px'},{width:'30px'}],{duration:100,fill:'both',composite:'add'});a.pause();a.currentTime=50;const b=e.animate([{width:'40px'},{width:'80px'}],{duration:100,fill:'both',composite:'add'});b.pause();b.currentTime=50;getComputedStyle(e).width").await.unwrap(),"180px");
    assert_eq!(
        page.evaluate(
            "a.effect.setKeyframes([{width:'30px'},{width:'50px'}]);getComputedStyle(e).width"
        )
        .await
        .unwrap(),
        "200px"
    );
    assert_eq!(
        page.evaluate("b.cancel();getComputedStyle(e).width")
            .await
            .unwrap(),
        "140px"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn animation_frame_callback_and_microtask_edits_reach_the_same_paint() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/frame-edits",
        "<!doctype html><article style='height:10px'></article>",
    )
    .await
    .unwrap();
    page.evaluate("globalThis.a=document.querySelector('article').animate([{width:'10px'},{width:'110px'}],{duration:100,fill:'both'});a.pause();a.currentTime=0;requestAnimationFrame(()=>{a.currentTime=50;queueMicrotask(()=>a.effect.setKeyframes([{width:'110px'},{width:'210px'}]))});undefined").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 160.);
}
#[tokio::test(flavor = "current_thread")]
async fn child_animation_frame_callback_updates_parent_sample_before_paint() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://animation.test/child-frame-edits",
        "<!doctype html><article style='height:10px'></article><iframe></iframe>",
    )
    .await
    .unwrap();
    page.evaluate("globalThis.a=document.querySelector('article').animate([{width:'10px'},{width:'110px'}],{duration:100,fill:'both'});a.pause();a.currentTime=0;frames[0].requestAnimationFrame(()=>{a.currentTime=75});undefined").await.unwrap();
    assert_eq!(article_width(&page.paint_snapshot().await.unwrap()), 85.);
}
