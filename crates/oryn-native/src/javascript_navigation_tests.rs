use super::*;
async fn wait_for(context: &BrowserContext, page: &PageHandle, expression: &str) {
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if page.evaluate(expression).await.unwrap() == "true" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    outcome.unwrap_or_else(|error| {
        panic!(
            "{expression}: {error}; pages={:?}",
            context
                .pages()
                .iter()
                .map(|entry| (
                    entry.page.page_id,
                    context.queued_navigation_status(entry.page.page_id)
                ))
                .collect::<Vec<_>>()
        )
    });
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_window_open_reports_blocking_on_source_without_throwing() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html(
        "https://script.test/",
        r#"<meta http-equiv=Content-Security-Policy content="script-src 'nonce-abc'"><body>source"#,
    )
    .await
    .unwrap();
    assert_eq!(page.evaluate("globalThis.events=[];addEventListener('securitypolicyviolation',e=>events.push([e.effectiveDirective,e.blockedURI]));globalThis.child=window.open('javascript:opener.ran=true');true").await.unwrap(), "true");
    wait_for(&context, &page, "events.length===1").await;
    assert_eq!(
        page.evaluate("JSON.stringify(events)").await.unwrap(),
        "[[\"script-src-elem\",\"inline\"]]"
    );
    assert_eq!(
        page.evaluate("globalThis.ran===undefined").await.unwrap(),
        "true"
    );
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_window_open_hash_and_nonstring_completion_preserve_document() {
    use sha2::Digest;
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    let source = "javascript:opener.postMessage('pass', '*')";
    let hash = BASE64.encode(sha2::Sha256::digest(source.as_bytes()));
    page.load_html("https://script.test/",format!(r#"<meta http-equiv=Content-Security-Policy content="script-src 'unsafe-hashes' 'nonce-abc' 'sha256-{hash}'"><body>source"#)).await.unwrap();
    page.evaluate(format!("globalThis.messages=[];onmessage=e=>messages.push(e.data);globalThis.child=window.open({});globalThis.first=child.document;true",serde_json::to_string(source).unwrap())).await.unwrap();
    wait_for(&context, &page, "messages.length===1").await;
    assert_eq!(
        page.evaluate(
            "messages[0]==='pass'&&child.document===first&&child.location.href==='about:blank'"
        )
        .await
        .unwrap(),
        "true"
    );
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_string_completion_replaces_document_with_inherited_origin() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html("https://script.test/source", "<body>source")
        .await
        .unwrap();
    let url = "javascript:'<title>replacement</title><p>new</p>'";
    page.evaluate(format!(
        "globalThis.child=window.open({});globalThis.first=child.document;true",
        serde_json::to_string(url).unwrap()
    ))
    .await
    .unwrap();
    wait_for(&context, &page, "child.document.title==='replacement'").await;
    assert_eq!(page.evaluate("child.document!==first&&child.document.body.textContent==='new'&&child.location.href==='about:blank'&&child.history.length===1").await.unwrap(),"true");
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_navigation_replacement_retains_csp() {
    use sha2::Digest;
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    let url = "javascript:'<title>replacement</title><script>opener.injected=true</script>'";
    let hash = BASE64.encode(sha2::Sha256::digest(url.as_bytes()));
    page.load_html("https://script.test/", format!(r#"<meta http-equiv=Content-Security-Policy content="script-src 'unsafe-hashes' 'sha256-{hash}'"><body>source"#)).await.unwrap();
    page.evaluate(format!(
        "globalThis.child=window.open({});true",
        serde_json::to_string(url).unwrap()
    ))
    .await
    .unwrap();
    wait_for(
        &context,
        &page,
        "child.document.title==='replacement'&&child.document.readyState==='complete'",
    )
    .await;
    assert_eq!(
        page.evaluate("globalThis.injected===undefined&&child.location.href==='about:blank'")
            .await
            .unwrap(),
        "true"
    );
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_navigation_object_and_exception_keep_document_without_eval() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html("https://script.test/", "<body>source")
        .await
        .unwrap();
    page.evaluate("globalThis.completed=0;globalThis.errors=0;globalThis.converted=false;globalThis.child=window.open();globalThis.first=child.document;child.document.body.textContent='kept';child.eval=()=>{throw Error('author eval')};child.onerror=()=>{errors++;return true};true").await.unwrap();
    let url = "javascript:opener.completed++;({toString(){opener.converted=true;return '<title>wrong</title>'}})";
    page.evaluate(format!(
        "child.location={};true",
        serde_json::to_string(url).unwrap()
    ))
    .await
    .unwrap();
    wait_for(&context, &page, "completed===1").await;
    assert_eq!(page.evaluate("!converted&&child.document===first&&child.document.body.textContent==='kept'&&child.history.length===1").await.unwrap(), "true");
    page.evaluate("child.location='javascript:throw null';true")
        .await
        .unwrap();
    wait_for(&context, &page, "errors===1").await;
    assert_eq!(
        page.evaluate("child.document===first&&child.document.body.textContent==='kept'")
            .await
            .unwrap(),
        "true"
    );
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_navigation_nested_navigation_cancels_string_replacement() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html("https://script.test/", "<body>source")
        .await
        .unwrap();
    page.evaluate("globalThis.child=window.open();globalThis.first=child.document;child.document.body.textContent='kept';true").await.unwrap();
    let url = "javascript:location.href='about:blank#after';'<title>wrong</title>'";
    page.evaluate(format!(
        "child.location={};true",
        serde_json::to_string(url).unwrap()
    ))
    .await
    .unwrap();
    wait_for(&context, &page, "child.location.href==='about:blank#after'").await;
    assert_eq!(
        page.evaluate("child.document===first&&child.document.body.textContent==='kept'")
            .await
            .unwrap(),
        "true"
    );
    page.evaluate("child.close();true").await.unwrap();
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn javascript_navigation_authority_rejects_changed_url_stale_and_foreign_targets() {
    let context = Browser::in_process_probe().new_context();
    let source = context.new_page();
    source
        .load_html("https://script.test/source", "<body>source")
        .await
        .unwrap();
    source
        .evaluate("globalThis.child=window.open();true")
        .await
        .unwrap();
    let target = context
        .wait_for_page(
            PagePredicate::Opener {
                page_id: source.page_id(),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    let source_creator = context
        .capture_command_navigation_source(source.page_id())
        .unwrap();
    let initial_target = context
        .capture_command_navigation_source(target.page_id())
        .unwrap();
    let url = "javascript:opener.marker=true";
    let issue = |target_creator| {
        context
            .issue_script_navigation_authority(ScriptNavigationInputs {
                source: source.page_id(),
                source_creator: source_creator.clone(),
                destination: target.page_id(),
                target_creator,
                destination_url: url,
                source_url: "https://script.test/source",
                referrer_policy: "",
            })
            .unwrap()
    };
    let authority = issue(initial_target.clone());
    assert!(
        context
            .script_navigation_network_source(&authority, target.page_id(), &initial_target, url)
            .is_ok()
    );
    assert!(
        context
            .script_navigation_network_source(
                &authority,
                target.page_id(),
                &initial_target,
                "javascript:other()"
            )
            .is_err()
    );
    target
        .load_html("https://foreign.test/", "<body>foreign")
        .await
        .unwrap();
    assert!(
        context
            .script_navigation_network_source(&authority, target.page_id(), &initial_target, url)
            .is_err()
    );
    let foreign_target = context
        .capture_command_navigation_source(target.page_id())
        .unwrap();
    let foreign_authority = issue(foreign_target.clone());
    assert!(matches!(
        context.script_navigation_network_source(
            &foreign_authority,
            target.page_id(),
            &foreign_target,
            url
        ),
        Err(RuntimeError::NavigationCancelled)
    ));
    assert!(matches!(
        context.prepare_javascript_navigation(
            target.page_id(),
            oryn_js::navigation::ScriptNavigationAdmission {
                authority: foreign_authority,
                target: foreign_target
            },
            url,
            "https://foreign.test/",
            "<body>injected".into()
        ),
        Err(RuntimeError::NavigationCancelled)
    ));
    assert_eq!(
        target.evaluate("document.body.textContent").await.unwrap(),
        "foreign"
    );
    target.close().await.unwrap();
    source.close().await.unwrap();
}
