#![cfg(all(unix, feature = "v8-host", feature = "in-process-probe"))]
use oryn_native::runtime::Browser;

fn bounded_window_diagnostic(args: std::fmt::Arguments<'_>) -> String {
    use std::fmt::Write;
    struct Buffer(String);
    impl std::fmt::Write for Buffer {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            let remaining = 1008usize.saturating_sub(self.0.len());
            let mut end = value.len().min(remaining);
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            self.0.push_str(&value[..end]);
            if end < value.len() {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }
    }
    let mut buffer = Buffer(String::with_capacity(1024));
    if buffer.write_fmt(args).is_err() {
        buffer.0.push_str(" [truncated]");
    }
    buffer.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_window_name_and_parent_selection_keep_exact_utf16() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html(
            "https://names.test/source",
            "<form id=f action='/submit' method=post><input name=x value=owned></form>",
        )
        .await
        .unwrap();
    assert_eq!(
        source
            .evaluate("window.open('','receiver');true")
            .await
            .unwrap(),
        "true"
    );
    let receiver = context
        .wait_for_page(
            oryn_common::v4::PagePredicate::Opener {
                page_id: source.page_id(),
            },
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    receiver
        .load_html("https://names.test/receiver", "<body>Receiver</body>")
        .await
        .unwrap();
    for expression in [
        "'abc\\0def'",
        "'n'+String.fromCharCode(0xd800)",
        "'n'+String.fromCharCode(0xdfff)",
        "String.fromCodePoint(0xf0000)+'oryn-wtf16:[55296]'",
        "'n'+String.fromCodePoint(0x1f600)",
    ] {
        receiver.evaluate(&format!("window.name={expression};globalThis.seen=[];navigation.onnavigate=e=>{{seen.push(e.formData.get('x'));e.preventDefault()}};window.name===({expression})")).await.map(|result|assert_eq!(result,"true")).unwrap();
        source
            .evaluate(&format!("f.target={expression};f.submit();true"))
            .await
            .unwrap();
        assert_eq!(receiver.evaluate("seen.join(',')").await.unwrap(), "owned");
        assert_eq!(
            receiver.evaluate("location.href").await.unwrap(),
            "https://names.test/receiver"
        );
    }
    receiver.close().await.unwrap();
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_open_preserves_names_conversion_order_and_feature_relationships() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html("https://names.test/source", "<body></body>")
        .await
        .unwrap();
    assert_eq!(source.evaluate(r#"(()=>{
        const check=(value,message)=>{if(!value)throw Error(message)};
        const nativeOpen=window.open;check(nativeOpen.length===0,'open length');
        const oldJSON=JSON.stringify,oldString=globalThis.String,oldQuery=document.querySelectorAll;
        JSON.stringify=()=>{throw Error('JSON bridge')};document.querySelectorAll=()=>{throw Error('DOM target query')};
        const name='target\n<'+String.fromCharCode(0xd800)+'\0'+String.fromCodePoint(0xf0000)+'oryn-wtf16:[1]';
        globalThis.String=()=>{throw Error('global String')};
        try{
            const order=[];const popup=nativeOpen.call(window,{toString(){order.push('url');return ''}},{toString(){order.push('target');return name}},{toString(){order.push('features');return 'noopener=0'}});
            check(popup&&order.join(',')==='url,target,features','conversion order');
            check(popup.name===name&&popup.opener===window,'initial native name and opener');
            check(nativeOpen.call(window,'',name)===popup,'exact named popup reuse');
            const other=nativeOpen.call(window,'',name+'x');check(other&&other!==popup,'distinct target');
            let after=false;const reason={};try{nativeOpen.call(window,{toString(){throw reason}},{toString(){after=true;return name}})}catch(error){check(error===reason,'original conversion exception')}
            check(!after,'conversion suffix stopped');
            check(nativeOpen.call(window,'',name,'noopener')===null,'noopener hides result');
            check(nativeOpen.call(window,'',name,'noreferrer=1,noopener=0')===null,'noreferrer implies noopener');
            check(nativeOpen.call(window,'','_self')===window,'self target');
            let invalid=false;try{nativeOpen.call(window,'http://[')}catch(error){invalid=error.name==='SyntaxError'}check(invalid,'invalid URL exception');
            popup.close();other.close();
        }finally{globalThis.String=oldString;JSON.stringify=oldJSON;document.querySelectorAll=oldQuery}
        return true;
    })()"#).await.unwrap(),"true");
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_names_and_borrowed_open_use_native_identity_after_renaming() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source.load_html("https://names.test/entry/page",r#"<base href="https://names.test/entry/"><iframe id=a srcdoc="<base href='https://names.test/child/'><p>A</p>"></iframe><iframe id=b srcdoc="<p>B</p>"></iframe>"#).await.unwrap();
    assert_eq!(source.evaluate(r#"(()=>{
        const check=(value,message)=>{if(!value)throw Error(message)};
        const first=a.contentWindow,second=b.contentWindow;
        const prefix=String.fromCodePoint(0xf0000)+'oryn-wtf16:';
        const name='frame'+String.fromCharCode(0xd800),other='frame'+String.fromCharCode(0xdfff);
        a.name=name;b.name=other;
        check(first.name===name&&second.name===other&&window[name]===first&&window[other]===second,'frame attribute names');
        first.name=prefix+'[55296]';check(window[prefix+'[55296]']===first&&window[name]===undefined,'child rename exact');
        check(open('',prefix+'[55296]')===first&&open('',other)===second,'native named lookup');
        check(first.open('','_self')===first&&first.open('','_parent')===window,'borrowed method receiver');
        const nested=first.document.createElement('iframe');first.document.body.append(nested);const inner=nested.contentWindow;
        check(inner.open('','_SELF')===inner&&inner.open('','_PARENT')===first&&inner.open('','_TOP')===window,'nested keyword ancestry');
        const popup=open('','duplicate-native-name');b.name='duplicate-native-name';nested.name='duplicate-native-name';
        check(inner.name==='duplicate-native-name','initial nested name cell');inner.name='renamed-from-window';check(first.open('','renamed-from-window')===inner,'initial child Window rename lookup');nested.name='duplicate-native-name';
        check(first.open('','duplicate-native-name')===inner,'source subtree before sibling and popup');
        nested.name='other-inner';check(first.open('','duplicate-native-name')===second,'source tree before popup');
        b.name='other-sibling';check(first.open('','duplicate-native-name')===popup,'other page after source tree');
        const popupFrame=popup.document.createElement('iframe');popup.document.body.append(popupFrame);void popupFrame.contentWindow.document;
        globalThis.closedPopup=popup;popup.close();check(closedPopup.closed,'script close acknowledgement');


        globalThis.destination='';
        const probe=globalThis.__windowTargetNavigationProbe={entered:0,assigned:false,prevented:false,error:null,scheduledAt:Date.now(),enteredAt:null};
        first.navigation.onnavigate=event=>{
            probe.entered++;
            probe.enteredAt=Date.now();
            try {
                destination=event.destination.url;
                probe.assigned=true;
                event.preventDefault();
                probe.prevented=true;
            } catch(error) {
                let message;
                try { message=String(error).slice(0,256); }
                catch { message='[exception stringification failed]'; }
                probe.error=message;
                throw error;
            }
        };
        first.open('relative','_self');return true;
    })()"#).await.unwrap(),"true");
    let polling_started = std::time::Instant::now();
    let mut polls_started = 0u64;
    let mut polls_completed = 0u64;
    let mut last_completion_ms = 0u128;
    let destination = match tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            polls_started += 1;
            let destination = source.evaluate("destination").await.unwrap();
            polls_completed += 1;
            last_completion_ms = polling_started.elapsed().as_millis();
            if !destination.is_empty() {
                break destination;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    {
        Ok(destination) => destination,
        Err(error) => {
            eprintln!(
                "window navigation terminal outcome: {}",
                bounded_window_diagnostic(format_args!(
                    "{:?}",
                    context.queued_navigation_status(source.page_id())
                ))
            );
            // Observe only after the original deadline. Do not retry navigation
            // or replace its destination/cancellation oracle with this probe.
            let probe = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                source.evaluate(
                    r#"JSON.stringify({
                callback:globalThis.__windowTargetNavigationProbe,
                destination:globalThis.destination.slice(0,256),
                rootURL:location.href.slice(0,256),
                childURL:a.contentWindow.location.href.slice(0,256),
                childReady:a.contentDocument.readyState,
                childEntry:a.contentWindow.navigation.currentEntry?.url.slice(0,256)??null,
                childOrigin:a.contentWindow.origin.slice(0,256),
                childClosed:a.contentWindow.closed,
                popupClosed:closedPopup.closed,
                hasHandler:typeof a.contentWindow.navigation.onnavigate
            })"#,
                ),
            )
            .await;
            panic!(
                "queued Window.open must deliver the destination's navigate event: {error}; polls started={polls_started} completed={polls_completed} last completion ms={last_completion_ms}; native custody={:?}; callback and target probe={}",
                context
                    .queued_navigation_status(source.page_id())
                    .map(|snapshot| bounded_window_diagnostic(format_args!("{snapshot:?}"))),
                bounded_window_diagnostic(format_args!("{probe:?}"))
            );
        }
    };
    assert_eq!(destination, "https://names.test/entry/relative");
    assert_eq!(source.evaluate("closedPopup.closed").await.unwrap(), "true");
    source.evaluate("a.remove();true").await.unwrap();
    assert_eq!(
        source
            .evaluate("b.contentWindow.opener===null")
            .await
            .unwrap(),
        "true"
    );
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_conversion_document_replacement_cannot_create_a_popup() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html("https://names.test/source", "<body>Original</body>")
        .await
        .unwrap();
    let before = context.pages().len();
    source.evaluate(r#"globalThis.openResult=open({toString(){document.open();return ''}},'retired-conversion',{toString(){return 'noopener=0'}});document.write('<body>Replacement</body>');document.close();true"#).await.unwrap();
    assert_eq!(source.evaluate("openResult===null").await.unwrap(), "true");
    assert_eq!(context.pages().len(), before);
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_named_frame_opener_survives_navigation_and_disowns_natively() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html(
            "https://names.test/source",
            r#"<iframe id=f name=existing-frame srcdoc="<p>Initial</p>"></iframe>"#,
        )
        .await
        .unwrap();
    assert_eq!(source.evaluate("globalThis.savedFrame=f.contentWindow;open('about:blank','existing-frame')===savedFrame").await.unwrap(),"true");
    assert_eq!(source.evaluate(r#"(()=>{
        if(f.contentWindow!==savedFrame||savedFrame.opener!==window)throw Error('named frame opener lost on navigation');
        globalThis.openerGetter=Object.getOwnPropertyDescriptor(savedFrame,'opener').get;
        if(openerGetter.call(savedFrame)!==window)throw Error('native opener getter');
        savedFrame.opener=null;savedFrame.opener='authored';
        if(openerGetter.call(savedFrame)!==null||savedFrame.opener!=='authored')throw Error('disown/replaceable separation');
        open('about:blank#next','existing-frame');return true;
    })()"#).await.unwrap(),"true");
    assert_eq!(
        source
            .evaluate("openerGetter.call(f.contentWindow)===window")
            .await
            .unwrap(),
        "true"
    );
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_page_service_starts_after_registration_and_closed_page_stays_rejected() {
    // Public context custody survives dropping the Browser value. The direct
    // service must see an admitted page before its first document command.
    let context = {
        let browser = Browser::in_process_probe();
        browser.new_context()
    };
    let closed = context.new_page();
    closed
        .load_html(
            "https://names.test/initial",
            "<!doctype html><body>registered</body>",
        )
        .await
        .unwrap();
    assert_eq!(
        closed
            .evaluate("document.body.textContent==='registered'")
            .await
            .unwrap(),
        "true"
    );
    let survivor = context.new_page();
    survivor
        .load_html(
            "https://names.test/survivor",
            "<!doctype html><body>live</body>",
        )
        .await
        .unwrap();
    closed.close().await.unwrap();
    assert!(matches!(
        closed.evaluate("throw Error('closed actor ran')").await,
        Err(oryn_native::runtime::RuntimeError::Closed)
    ));
    assert_eq!(
        survivor
            .evaluate("document.body.textContent==='live'")
            .await
            .unwrap(),
        "true"
    );
    survivor.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_popup_accessors_retain_exact_names_and_driver_root_opener_identity() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html("https://names.test/source", "<body></body>")
        .await
        .unwrap();
    assert_eq!(source.evaluate(r#"(()=>{
        const originalName='initial-'+String.fromCharCode(0xd800)+'\0';
        const renamed='renamed-'+String.fromCharCode(0xdc00);
        const popup=open('',originalName,'noopener=0');
        if(!popup||popup.name!==originalName)throw Error('parent-owned initial target name');
        if(popup.opener!==window)throw Error('driver root WindowProxy identity');
        const getter=Object.getOwnPropertyDescriptor(popup,'name').get;
        const setter=Object.getOwnPropertyDescriptor(popup,'name').set;
        setter.call(popup,renamed);
        if(getter.call(popup)!==renamed||open('',renamed)!==popup)throw Error('parent target registry rename');
        if(window.name!=='')throw Error('borrowed getter or setter changed caller');
        popup.opener=null;
        if(popup.opener!==null)throw Error('native disowning');
        popup.close();return true;
    })()"#).await.unwrap(),"true");
    source.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_srcdoc_authority_is_published_before_child_window_operations() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source.load_html("https://names.test/source",r#"<iframe id=child name=published srcdoc="<script>globalThis.derived=document.cloneNode(false).nodeType===9;globalThis.selfSelected=open('', '_self')===window;globalThis.parentSelected=open('', '_parent')===parent;</script>"></iframe>"#).await.unwrap();
    assert_eq!(source.evaluate(r#"(()=>{
        const child=document.getElementById('child').contentWindow;
        if(!child.derived||!child.selfSelected||!child.parentSelected)throw Error('child authority unavailable during script');
        return open('', 'published')===child;
    })()"#).await.unwrap(),"true");
    source.close().await.unwrap();
}
