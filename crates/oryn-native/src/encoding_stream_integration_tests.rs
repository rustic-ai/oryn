//! Native iframe retirement and constructor-realm encoding-stream obligations.
use super::*;

// PageHandle::evaluate stringifies its immediate value; it does not await a
// returned Promise. Retain the outcome and poll native turns to assert the
// actual async stream results (including constructor-realm rejections).
async fn assert_async_true(page: &PageHandle, source: &str) {
    page.evaluate(format!("globalThis.__encodingResult='pending';Promise.resolve({source}).then(value=>globalThis.__encodingResult=value===true?'passed':'failed',error=>globalThis.__encodingResult='rejected: '+String(error));void 0")).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let result = page.evaluate("globalThis.__encodingResult").await.unwrap();
        if result != "pending" {
            assert_eq!(result, "passed");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "encoding stream promise did not settle"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn reachable_encoding_streams_continue_after_native_iframe_removal() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://encoding.test/retained",
        "<!doctype html><iframe></iframe>",
    )
    .await
    .unwrap();
    assert_async_true(&page,r#"(async()=>{
      const frame=document.querySelector('iframe'), realm=frame.contentWindow;
      const encoder=new realm.TextEncoderStream(),decoder=new realm.TextDecoderStream();
      const er=encoder.readable.getReader(),ew=encoder.writable.getWriter();
      const dr=decoder.readable.getReader(),dw=decoder.writable.getWriter();
      const encoded=er.read(),decoded=dr.read();await Promise.all([ew.ready,dw.ready]);
      frame.remove();
      await Promise.all([ew.write('A'),dw.write(new Uint8Array([65]))]);
      const e=await encoded,d=await decoded;
      if(!(e instanceof realm.Object)||!(d instanceof realm.Object)||!(e.value instanceof realm.Uint8Array)||e.value[0]!==65||d.value!=='A')return false;
      const endE=er.read(),endD=dr.read();await Promise.all([ew.close(),dw.close()]);
      return (await endE).done&&(await endD).done;
    })()"#).await;
}

#[tokio::test(flavor = "current_thread")]
async fn encoding_stream_conversion_can_remove_its_constructor_frame() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://encoding.test/reentrant",
        "<!doctype html><iframe></iframe>",
    )
    .await
    .unwrap();
    assert_async_true(&page,r#"(async()=>{
      const frame=document.querySelector('iframe'),realm=frame.contentWindow;
      const stream=new realm.TextEncoderStream(),reader=stream.readable.getReader(),writer=stream.writable.getWriter();
      const pending=reader.read();await writer.write({toString(){frame.remove();return '\uD83D\uDE00'}});
      const result=await pending;
      if(!(result instanceof realm.Object)||!(result.value instanceof realm.Uint8Array)||String(result.value)!=='240,159,152,128')return false;
      const end=reader.read();await writer.close();return (await end).done;
    })()"#).await;
}

#[tokio::test(flavor = "current_thread")]
async fn detached_decoder_errors_belong_to_constructor_realm() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://encoding.test/errors",
        "<!doctype html><iframe></iframe>",
    )
    .await
    .unwrap();
    assert_async_true(&page,r#"(async()=>{
      const frame=document.querySelector('iframe'),realm=frame.contentWindow;
      const invalid=new realm.TextDecoderStream('utf-8',{fatal:true});
      const reader=invalid.readable.getReader(),writer=invalid.writable.getWriter();
      const read=reader.read().then(()=>false,e=>e instanceof realm.TypeError);
      frame.remove();const write=writer.write(new Uint8Array([255])).then(()=>false,e=>e instanceof realm.TypeError);
      return (await read)&&(await write);
    })()"#).await;
}

#[test]
fn legacy_background_hints_use_document_charset_without_rewriting_selector_attributes() {
    for (encoding, suffix) in [
        ("UTF-8", "%C3%BF"),
        ("windows-1251", "%26%23255%3B"),
        ("windows-1252", "%FF"),
        ("UTF-16LE", "%C3%BF"),
    ] {
        let mut document = crate::html::parse(
            "<!doctype html><style>td[background='?ÿ']{color:rgb(1,2,3)}th{background-image:none}</style><body background='?ÿ'><table background='?ÿ'><thead background='?ÿ'><tr background='?ÿ'><th background='?ÿ'>x</th></tr></thead><tbody background='?ÿ'><tr><td background='?ÿ'>x</td></tr></tbody><tfoot background='?ÿ'><tr><td>x</td></tr></tfoot></table><div background='?ÿ'></div>",
        );
        let metadata = document
            .dom
            .get_mut(document.document)
            .unwrap()
            .document_metadata
            .get_or_insert_with(|| Box::new(oryn_dom::DocumentMetadata::html()));
        metadata.url = Some("https://encoding.test/base/page".into());
        metadata.encoding = encoding.into();
        let input = layout_input(
            &document,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            &[],
        );
        let output = oryn_layout::compute(&input, &Default::default()).unwrap();
        for node in input
            .nodes
            .iter()
            .filter(|node| node.attributes.contains_key("background"))
        {
            assert_eq!(node.attributes["background"], "?ÿ");
            if node.tag == "div" || node.tag == "th" {
                assert_eq!(output.computed_styles[&node.id]["background-image"], "none");
            } else {
                assert!(
                    output.computed_styles[&node.id]["background-image"]
                        .ends_with(&format!("?{suffix}\")")),
                    "{} {}",
                    encoding,
                    node.tag
                );
            }
            if node.tag == "td" {
                assert_eq!(output.computed_styles[&node.id]["color"], "rgb(1, 2, 3)");
            }
        }
        let mut updated = input.clone();
        for url in updated.legacy_background_urls.values_mut() {
            *url = "https://other.test/new.png".into();
        }
        assert_ne!(updated, input);
        let output = oryn_layout::compute(&updated, &Default::default()).unwrap();
        let body = updated
            .nodes
            .iter()
            .find(|node| node.tag == "body")
            .unwrap();
        assert!(
            output.computed_styles[&body.id]["background-image"]
                .contains("https://other.test/new.png")
        );
    }
}
