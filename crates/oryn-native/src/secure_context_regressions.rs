//! Real native realm/ancestor/worker exposure coverage for the Core API gap.
use super::*;
use crate::host::webdriver_script::{ScriptRequest, ScriptValue};

struct Fixture;
impl crate::network::NetworkBroker for Fixture {
    fn send(
        &self,
        request: crate::network::NetworkRequest,
    ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
        let (content_type, body) = if request.url.ends_with("/worker.js") {
            (
                "text/javascript",
                r#"onmessage=()=>postMessage([isSecureContext,URL.canParse('/x','https://base.test/'),URL.parse('/x','https://base.test/').href]);"#,
            )
        } else if request.url.ends_with("/shared.js") {
            (
                "text/javascript",
                r#"addEventListener('connect',event=>{const port=event.ports[0];port.onmessage=()=>port.postMessage([isSecureContext,URL.canParse('/x','https://base.test/'),URL.parse('/x','https://base.test/').href]);port.start()});"#,
            )
        } else if request.url.ends_with("/popup") {
            (
                "text/html",
                r#"<script>opener.postMessage({kind:'secure-popup',secure:isSecureContext},'*')</script>"#,
            )
        } else if request.url.ends_with("/child") {
            (
                "text/html",
                r#"<script>parent.postMessage({kind:'secure-child',secure:isSecureContext},'*')</script>"#,
            )
        } else {
            ("text/html", "<body>Native secure-context fixture</body>")
        };
        Ok(crate::network::NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: request.request_id,
            status: 200,
            headers: vec![("content-type".into(), content_type.into())],
            body: body.as_bytes().to_vec(),
        })
    }
}
async fn script(page: &PageHandle, body: &str) -> ScriptValue {
    page.execute_webdriver_script(ScriptRequest {
        body: body.encode_utf16().collect(),
        args: Vec::new(),
        asynchronous: false,
        timeout_ms: Some(5000),
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_secure_context_respects_ancestors_inheritance_and_cross_origin_windows() {
    for (url, secure) in [
        ("http://ancestor.test/", false),
        ("https://ancestor.test/", true),
    ] {
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), SharedNetworkBroker::new(Fixture));
        let page = context.new_page();
        page.goto(url).await.unwrap();
        assert_eq!(
            page.evaluate("String(isSecureContext)").await.unwrap(),
            secure.to_string()
        );
        assert_eq!(script(&page, "const frame=document.createElement('iframe');document.body.append(frame);const answer=frame.contentWindow.isSecureContext;frame.remove();return answer").await, ScriptValue::Bool(secure));
        assert_eq!(script(&page, r#"return new Promise(resolve=>{const frame=document.createElement('iframe');frame.srcdoc='<body>inherited';frame.onload=()=>{const answer=frame.contentWindow.isSecureContext;frame.remove();resolve(answer)};document.body.append(frame)})"#).await, ScriptValue::Bool(secure));
        assert_eq!(script(&page, r#"return new Promise(resolve=>{
            const frame=document.createElement('iframe');frame.sandbox='allow-scripts';
            const listener=event=>{if(event.source!==frame.contentWindow||event.data.kind!=='opaque-secure')return;removeEventListener('message',listener);frame.remove();resolve(event.data.secure)};
            addEventListener('message',listener);
            frame.srcdoc='<script>parent.postMessage({kind:"opaque-secure",secure:isSecureContext},"*")<\/script>';
            document.body.append(frame);
        })"#).await, ScriptValue::Bool(secure));
        let report = script(&page, r#"return new Promise(resolve=>{
            const frame=document.createElement('iframe');
            const listener=event=>{
                if(event.source!==frame.contentWindow||event.data.kind!=='secure-child')return;
                removeEventListener('message',listener);
                let direct=false,borrowed=false;
                try{void frame.contentWindow.isSecureContext}catch(error){direct=error.name==='SecurityError'}
                try{Object.getOwnPropertyDescriptor(globalThis,'isSecureContext').get.call(frame.contentWindow)}catch(error){borrowed=error.name==='SecurityError'}
                frame.remove();resolve([event.data.secure,direct,borrowed]);
            };
            addEventListener('message',listener);frame.src='https://child.test/child';document.body.append(frame);
        })"#).await;
        assert_eq!(
            report.to_json_text().unwrap(),
            format!("[{secure},true,true]")
        );
        page.close().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_secure_context_popup_uses_initial_inheritance_then_its_own_top_level_commit() {
    let context = Browser::in_process_with_config(Default::default())
        .new_context_with_broker(Default::default(), SharedNetworkBroker::new(Fixture));
    let page = context.new_page();
    for (url, secure) in [
        ("http://opener.test/", false),
        ("https://opener.test/", true),
        ("http://opener.test/replaced", false),
    ] {
        page.goto(url).await.unwrap();
        assert_eq!(
            script(&page, "return isSecureContext").await,
            ScriptValue::Bool(secure)
        );
        assert_eq!(script(&page, "const popup=window.open('about:blank');const answer=popup.isSecureContext;popup.close();return answer").await, ScriptValue::Bool(secure));
    }
    assert_eq!(script(&page, r#"return new Promise(resolve=>{
        let popup;
        const listener=event=>{if(event.source!==popup||event.data.kind!=='secure-popup')return;removeEventListener('message',listener);const answer=event.data.secure;popup.close();resolve(answer)};
        addEventListener('message',listener);popup=window.open('https://popup.test/popup');
    })"#).await, ScriptValue::Bool(true), "insecure opener is not an ancestor of a committed secure popup");
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_worker_secure_context_and_url_statics_use_committed_worker_globals() {
    for (url, secure) in [
        ("http://worker.test/", false),
        ("https://worker.test/", true),
    ] {
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), SharedNetworkBroker::new(Fixture));
        let page = context.new_page();
        page.goto(url).await.unwrap();
        let dedicated = script(
            &page,
            r#"return new Promise((resolve,reject)=>{
            const worker=new Worker('/worker.js');
            worker.onmessage=event=>{worker.terminate();resolve(event.data)};
            worker.onerror=event=>{worker.terminate();reject(Error(event.message))};
            worker.postMessage('probe');
        })"#,
        )
        .await;
        assert_eq!(
            dedicated.to_json_text().unwrap(),
            format!("[{secure},true,\"https://base.test/x\"]")
        );
        let shared = script(
            &page,
            r#"return new Promise((resolve,reject)=>{
            const worker=new SharedWorker('/shared.js','secure-context-gap');
            worker.port.onmessage=event=>{worker.port.close();resolve(event.data)};
            worker.onerror=event=>{worker.port.close();reject(Error(event.message))};
            worker.port.start();worker.port.postMessage('probe');
        })"#,
        )
        .await;
        assert_eq!(
            shared.to_json_text().unwrap(),
            format!("[{secure},true,\"https://base.test/x\"]")
        );
        page.close().await.unwrap();
    }
}
