#![cfg(all(feature = "v8-host", feature = "in-process-probe"))]
//! Pinned runScripts observes the complete replacement through real layout.
use oryn_native::runtime::Browser;

#[tokio::test(flavor = "current_thread")]
async fn fragment_scripts_observe_final_light_and_shadow_style() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html(
        "https://sink-style.test/page",
        "<!doctype html><body></body>",
    )
    .await
    .unwrap();
    page.evaluate(r##"(()=>{
        for(const shadow of [false,true]){
            const host=document.createElement('div');document.body.append(host);
            const target=shadow?host.attachShadow({mode:'open'}):host;
            globalThis.sinkScope=target;globalThis.sinkColors=[];
            target.setHTMLUnsafe('<span id="sink-target">text</span><style>#sink-target{color:rgb(0,128,0)}</style><script>sinkColors.push(getComputedStyle(sinkScope.querySelector("#sink-target")).color)<'+ '/script><style>#sink-target{color:rgb(0,0,128)}</style><script>sinkColors.push(getComputedStyle(sinkScope.querySelector("#sink-target")).color)<'+ '/script>',{runScripts:true});
            if(sinkColors.length!==2||sinkColors.some(value=>value!=='rgb(0, 0, 128)'))throw Error('partial style visible: '+sinkColors);
            host.remove();
        }
        delete globalThis.sinkScope;delete globalThis.sinkColors;return true;
    })()"##).await.unwrap();
    page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn declarative_css_modules_share_identity_clone_and_preserve_source_attributes() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html("https://sink-style.test/page",r#"<!doctype html><script type="importmap">{"imports":{"red":"data:text/css,span%7Bcolor:red%7D","green":"data:text/css,span%7Bcolor:green%7D"}}</script><body></body>"#).await.unwrap();
    page.evaluate(r#"(()=>{
        const check=(v,m)=>{if(!v)throw Error(m)};
        document.body.setHTMLUnsafe('<div id="outer"><template shadowrootmode="open" shadowrootclonable shadowrootserializable shadowrootadoptedstylesheets=" red   red  "><span id="r">red</span><div id="inner"><template shadowrootmode="open" shadowrootadoptedstylesheets="green"><span id="g">green</span></template></div></template></div>');
        const host=document.getElementById('outer'),root=host.shadowRoot,inner=root.getElementById('inner').shadowRoot;
        check(root.adoptedStyleSheets.length===2&&root.adoptedStyleSheets[0]===root.adoptedStyleSheets[1],'duplicate specifier shares module identity');
        check(getComputedStyle(root.getElementById('r')).color==='rgb(255, 0, 0)'&&getComputedStyle(inner.getElementById('g')).color==='rgb(0, 128, 0)','independent nested root styles');
        const clone=host.cloneNode(true);check(clone.shadowRoot.adoptedStyleSheets[0]===root.adoptedStyleSheets[0],'clone reuses module map');
        const own=new CSSStyleSheet();own.replaceSync('span{font-weight:bold}');root.adoptedStyleSheets.push(own);
        const html=host.getHTML({serializableShadowRoots:true});check(html.includes('shadowrootadoptedstylesheets=" red   red  "')&&!html.includes('font-weight'),'original attribute survives script mutation');
        // getHTML serializes the host's contents, so restore its containing host for document parsing.
        const inert=Document.parseHTMLUnsafe('<div id=roundtrip>'+html+'</div>');check(inert.getElementById('roundtrip').shadowRoot.adoptedStyleSheets.length===0,'inert document cannot use caller module map');
        for(const raw of [null,'','  red   green  ']){
            const item=document.createElement('div');item.setHTMLUnsafe('<template shadowrootmode="open" shadowrootserializable'+(raw===null?'':' shadowrootadoptedstylesheets="'+raw+'"')+'></template>');
            const serialized=item.getHTML({serializableShadowRoots:true});check(raw===null?!serialized.includes('shadowrootadoptedstylesheets'):serialized.includes('shadowrootadoptedstylesheets="'+raw+'"'),'absent versus empty/raw preservation');
        }
        const template=document.createElement('template');check(template.shadowRootAdoptedStyleSheets==='','default reflection');template.shadowRootAdoptedStyleSheets=' red  green ';check(template.getAttribute('shadowrootadoptedstylesheets')===' red  green ','reflection');template.removeAttribute('shadowrootadoptedstylesheets');check(template.shadowRootAdoptedStyleSheets==='','removed reflection');
        return true;
    })()"#).await.unwrap();
    page.close().await.unwrap();
}
