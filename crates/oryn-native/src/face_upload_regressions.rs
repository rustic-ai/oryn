//! Native selection-to-request coverage; durable server receipts are covered by
//! the separate forms v4 FACE workflow case, not inferred from this broker.
use super::*;
use crate::network::{
    NetworkBroker, NetworkError, NetworkRequest, NetworkResponse, SharedNetworkBroker,
};
struct Capture(Arc<Mutex<Vec<NetworkRequest>>>);
impl NetworkBroker for Capture {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let request_id = request.request_id;
        self.0.lock().unwrap().push(request);
        Ok(NetworkResponse {
            timing: None,
            request_id,
            status: 200,
            headers: vec![("content-type".into(), "text/html".into())],
            body: b"<title>Receipt</title>Received".to_vec(),
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_file_survives_grant_revocation_face_snapshot_and_reset_after_acceptance() {
    let mut options = ContextOptions::default();
    options.action_policy.rules.insert(
        ConsequentialAction::FileUpload,
        crate::security::PolicyRule::Allow,
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let context = Browser::in_process_probe()
        .new_context_with_broker(options, SharedNetworkBroker::new(Capture(requests.clone())));
    let page = context.new_page();
    page.load_html("https://forms.test/start",r#"<form id=f action=/submit method=post enctype=multipart/form-data><input id=file type=file><x-upload id=x></x-upload></form><script>
    customElements.define('x-upload',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();}formResetCallback(){this.i.setFormValue(null);}});
    file.onchange=()=>{const data=new FormData();data.append('duplicate','first');data.append('document',file.files[0]);data.append('duplicate','second');x.i.setFormValue(data);data.delete('document');};
    </script>"#).await.unwrap();
    let payload = vec![0, 255, 13, 10, 128, 65];
    let handle = context.import_upload("proof.bin", "application/octet-stream", payload.clone());
    let target = page
        .resolve_selector("#file", SemanticAction::Click)
        .await
        .unwrap();
    page.set_files(target, &[handle], None).await.unwrap();
    assert!(context.revoke_upload(handle));
    page.evaluate("f.submit();f.reset();true").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !requests.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert!(body.windows(payload.len()).any(|bytes| bytes == payload));
    let text = String::from_utf8_lossy(body);
    assert!(text.contains("filename=\"proof.bin\""));
    assert!(text.find("first").unwrap() < text.find("second").unwrap());
    assert!(!text.contains("OPAQUE") && !text.contains("$file:"));
    drop(requests);
    page.close().await.unwrap();
}
#[tokio::test]
async fn oil_upload_policy_uses_selected_frame_origin_before_materialization() {
    use oryn_js::message_wire::MessageRequest;
    let mut options = ContextOptions::default();
    options.action_policy.rules.insert(
        ConsequentialAction::FileUpload,
        crate::security::PolicyRule::Allow,
    );
    options
        .action_policy
        .denied_origins
        .insert("https://cross.test".into());
    let context = Browser::in_process_probe().new_context_with_options(options);
    let page = context.new_page();
    let id = page.page_id();
    context
        .install_document_security(id, "https://top.test/".into(), &[])
        .unwrap();
    context
        .handle_message_wire(
            id,
            MessageRequest::RegisterRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
        )
        .unwrap();
    let token = "a".repeat(64);
    let url = "https://cross.test/frame";
    context
        .handle_message_wire(
            id,
            MessageRequest::RegisterWindow {
                frame: token.clone(),
                document_generation: 1,
                stream_epoch: 0,
                origin: "https://cross.test".into(),
                inherit_origin: None,
            },
        )
        .unwrap();
    let policy = context
        .prepare_frame_worker_policy(id, url, &[], "<input type=file>")
        .unwrap();
    context
        .bind_frame_worker_policy(id, token.clone(), 1, 0, url, &policy)
        .unwrap();
    let mut snapshot = crate::host::webdriver_script::AutomationRealmSnapshot {
        realm: oryn_js::realm::RealmId(1),
        parent_realm: Some(oryn_js::realm::RealmId(0)),
        frame: None,
        frame_generation: Some(1),
        frame_token: Some(token),
        document_root: oryn_dom::DomRuntime::new().document(),
        document_generation: 1,
        stream_epoch: 0,
        document_authority_epoch: 0,
        url: "https://forged-top.test/".into(),
        live: true,
    };
    let handle = context.import_upload("proof", "text/plain", b"private".to_vec());
    context.revoke_upload(handle);
    let target = SemanticRef {
        page_id: id,
        document_generation: 1,
        node: 1,
    };
    assert!(
        matches!(
            page.with_frame_scope(Some(snapshot.clone()))
                .set_files(target, &[handle], None)
                .await,
            Err(RuntimeError::PolicyDenied {
                action: ConsequentialAction::FileUpload
            })
        ),
        "selected denied origin precedes revoked-grant lookup"
    );
    snapshot.document_generation = 2;
    assert!(
        page.with_frame_scope(Some(snapshot.clone()))
            .set_files(target, &[handle], None)
            .await
            .is_err()
    );
    snapshot.document_generation = 1;
    snapshot.live = false;
    assert!(matches!(
        page.with_frame_scope(Some(snapshot))
            .set_files(target, &[handle], None)
            .await,
        Err(RuntimeError::StaleSemanticRef)
    ));
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_and_text_oil_upload_resolve_file_button_without_text_input_capability() {
    let mut options = ContextOptions::default();
    options.action_policy.rules.insert(
        ConsequentialAction::FileUpload,
        crate::security::PolicyRule::Allow,
    );
    let context = Browser::in_process_probe().new_context_with_options(options);
    let page = context.new_page();
    page.load_html("https://upload.test/","<label>Authorized document<input id=file type=file></label><script>globalThis.clicks=0;file.onclick=()=>clicks++</script>").await.unwrap();
    context.import_upload_as(
        "proof",
        "proof.bin",
        "application/octet-stream",
        vec![0, 255],
    );
    assert!(
        page.resolve_selector("#file", SemanticAction::Type)
            .await
            .is_err()
    );
    page.perform(oryn_common::agent::Intent::Upload {
        target: oryn_common::agent::Target::Css {
            selector: "#file".into(),
        },
        files: vec!["proof".into()],
    })
    .await
    .unwrap();
    let mut oil = crate::oil::NativeContextOilSession::new(context.clone());
    oil.execute("upload \"Authorized document\" $file:proof")
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("file.files.length===1&&file.files[0].size===2&&clicks===0")
            .await
            .unwrap(),
        "true"
    );
    page.close().await.unwrap();
}
