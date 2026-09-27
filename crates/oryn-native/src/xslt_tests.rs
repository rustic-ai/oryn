//! Execution-level XSLT witnesses: same native realm, document publication and
//! resource adapters used by ordinary navigation and connected script insertion.
use super::*;
use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};
use std::sync::{Arc, Mutex};

struct XsltBroker {
    requests: Arc<Mutex<Vec<NetworkRequest>>>,
    resources: BTreeMap<String, (String, String)>,
}
impl NetworkBroker for XsltBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let request_id = request.request_id;
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        let (mime, body) =
            self.resources.get(&url).cloned().ok_or_else(|| {
                NetworkError::Transport(format!("unexpected XSLT resource: {url}"))
            })?;
        Ok(NetworkResponse {
            request_id,
            status: 200,
            headers: vec![("content-type".into(), mime)],
            body: body.into_bytes(),
            response_policy: None,
            timing: None,
        })
    }
}
fn pump_until(page: &mut RustExecutableDocument, predicate: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        page.poll_context_events().unwrap();
        if page.agent.host.evaluate_string(predicate).unwrap() == "true" {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "XSLT execution did not complete: {predicate}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[test]
fn xslt_fragment_preserves_output_document_and_executes_internal_and_external_inserted_scripts() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut page = RustExecutableDocument::load_with_environment(
        "<body></body>",
        &BTreeMap::new(),
        Some(SharedNetworkBroker::new(XsltBroker {
            requests: requests.clone(),
            resources: BTreeMap::from([(
                "https://example.test/externalScript.js".into(),
                (
                    "text/javascript".into(),
                    "window.externalScript=true;window.externalRuns=(window.externalRuns||0)+1"
                        .into(),
                ),
            )]),
        })),
        Some(Url::parse("https://example.test/source.xml").unwrap()),
    )
    .unwrap();
    page.agent.host.execute(r#"
        const parser=new DOMParser(), processor=new XSLTProcessor();
        processor.importStylesheet(parser.parseFromString(`<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:output method="html"/><xsl:template match="/"><section id="transformed"><script>window.internalScript=true;window.internalRuns=(window.internalRuns||0)+1;</script><script src="externalScript.js"></script></section></xsl:template></xsl:stylesheet>`,"text/xml"));
        const fragment=processor.transformToFragment(parser.parseFromString('<source/>','text/xml'),document);
        if(fragment.ownerDocument!==document || fragment.firstChild.ownerDocument!==document || window.internalScript || window.externalScript)throw Error('fragment ownership or early execution');
        document.body.appendChild(fragment);
    "#).unwrap();
    pump_until(
        &mut page,
        "String(window.internalScript===true && window.externalScript===true)",
    );
    assert_eq!(page.agent.host.evaluate_string("String(internalRuns===1 && externalRuns===1 && document.getElementById('transformed').ownerDocument===document)").unwrap(),"true");
    assert_eq!(requests.lock().unwrap().len(), 1);
}
#[test]
fn xslt_parameters_reset_document_results_and_private_array_prototypes() {
    let mut page = RustExecutableDocument::load("<body></body>").unwrap();
    assert_eq!(page.agent.host.evaluate_string(r#"(()=>{
      let reentered=0;
      Object.defineProperty(Array.prototype,'0',{configurable:true,get(){reentered++;throw Error('private read')},set(){reentered++;throw Error('private write')}});
      try {
        const parser=new DOMParser(),processor=new XSLTProcessor();
        const xml=parser.parseFromString('<items/>','text/xml');
        const style=parser.parseFromString(`<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:param" xmlns:_oryn_parameter_0_0="urn:collision"><xsl:output method="text"/><xsl:param name="p:label" select="'default'"/><xsl:template match="*"><xsl:value-of select="$p:label"/><xsl:value-of select="name(.)"/></xsl:template></xsl:stylesheet>`,'text/xml');
        processor.importStylesheet(style);processor.setParameter('urn:param','label','custom:');
        const fragment=processor.transformToFragment(xml.documentElement,document);
        if(fragment.textContent!=='custom:items'||processor.getParameter('urn:param','label')!=='custom:')throw Error('parameters');
        processor.removeParameter('urn:param','label');
        if(processor.getParameter('urn:param','label')!==null)throw Error('remove');
        const outputDocument=parser.parseFromString('<output/>','text/xml');
        const detached=processor.transformToFragment(xml,outputDocument);
        if(detached.ownerDocument!==outputDocument||detached.firstChild.ownerDocument!==outputDocument)throw Error('detached output owner');
        const result=processor.transformToDocument(xml);
        if(result===document||result.documentElement.ownerDocument!==result||!result.documentElement.textContent.includes('defaultitems'))throw Error('document ownership');
        processor.setParameter('\uD800','\uDC00','\uD800\uE000');
        if(processor.getParameter('\uD800','\uDC00')!=='\uD800\uE000')throw Error('DOMString storage');
        processor.removeParameter('\uD800','\uDC00');if(processor.getParameter('\uD800','\uDC00')!==null)throw Error('DOMString removal');
        const sentinel=String.fromCodePoint(0xF0000)+'oryn-wtf16:[65]';processor.setParameter(null,'sentinel',sentinel);
        if(processor.getParameter(null,'sentinel')!==sentinel)throw Error('sentinel literal');
        processor.setParameter(null,'plain','value');processor.clearParameters();
        if(processor.getParameter(null,'plain')!==null)throw Error('clear');
        processor.reset();if(processor.transformToDocument(xml)!==null)throw Error('reset');
      } finally {delete Array.prototype[0];}
      return String(reentered===0);
    })()"#).unwrap(),"true");
}
#[test]
fn xslt_pi_navigation_uses_same_origin_destination_and_suppresses_discarded_source_script() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stylesheet = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:include href="parts/body.xsl"/></xsl:stylesheet>"#;
    let included = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:output method="html"/><xsl:template match="/"><html><head/><body><p id="result"><xsl:value-of select="/*/@value"/></p><script>window.transformedScript=true</script></body></html></xsl:template></xsl:stylesheet>"#;
    let broker = XsltBroker {
        requests: requests.clone(),
        resources: BTreeMap::from([
            (
                "https://example.test/main.xsl".into(),
                ("application/xslt+xml".into(), stylesheet.into()),
            ),
            (
                "https://example.test/parts/body.xsl".into(),
                ("application/xslt+xml".into(), included.into()),
            ),
        ]),
    };
    let source = r#"<?xml-stylesheet type="text/xsl" href="main.xsl"?><html xmlns="http://www.w3.org/1999/xhtml" value="transformed"><script>window.discardedSourceScript=true</script><body/></html>"#;
    let stream = crate::host::DocumentStream {
        content_language: None,
        chunks: Box::new(crate::host::ReadyDocumentInput(std::iter::once(Ok(
            source.into()
        )))),
        content_type: Some("application/xhtml+xml".into()),
        encoding: Arc::new(Mutex::new(Some("UTF-8"))),
    };
    let mut page = RustExecutableDocument::load_stream_with_limits(
        stream,
        &BTreeMap::new(),
        Some(SharedNetworkBroker::new(broker)),
        Some(Url::parse("https://example.test/input.xml").unwrap()),
        256 * 1024 * 1024,
        true,
    )
    .unwrap();
    page.agent
        .host
        .execute("globalThis.originalDocument=document")
        .unwrap();
    pump_until(
        &mut page,
        "String(globalThis.transformedScript===true && document.readyState!=='loading')",
    );
    assert_eq!(page.agent.host.evaluate_string("String(originalDocument===document && !globalThis.discardedSourceScript && document.getElementById('result').textContent==='transformed' && document.URL==='https://example.test/input.xml')").unwrap(),"true");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request.method, "GET");
        assert!(request.body.is_empty());
        assert_eq!(
            request.security.fetch_metadata,
            crate::fetch_metadata::Metadata::resource("same-origin", "xslt", false)
        );
    }
}

#[cfg(test)]
mod engine_cases {
    use super::*;
    #[derive(Default)]
    struct Resources(Vec<(String, String, String)>);
    fn validate(style: &str, url: &str, resources: &Resources) -> bool {
        crate::xslt_process::run(
            oryn_js::platform::XsltRequest {
                style: style.into(),
                source: None,
                parameters: vec![],
                url: url.into(),
                resources: resources.0.clone(),
            },
            || false,
        )
        .unwrap()
        .valid
    }
    fn transform(
        style: &str,
        source: &str,
        parameters: &[(String, String, String)],
        url: &str,
        resources: &Resources,
    ) -> Option<(String, i32)> {
        crate::xslt_process::run(
            oryn_js::platform::XsltRequest {
                style: style.into(),
                source: Some(source.into()),
                parameters: parameters.to_vec(),
                url: url.into(),
                resources: resources.0.clone(),
            },
            || false,
        )
        .unwrap()
        .output
    }
    fn sheet(body: &str) -> String {
        format!(
            "<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'>{body}</xsl:stylesheet>"
        )
    }
    #[test]
    fn xslt_templates_sort_parameters_and_namespaces_are_not_fixture_specific() {
        let style = "<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform' xmlns:p='urn:parameter' xmlns:_oryn_parameter_0_0='urn:collision'><xsl:output method='text'/><xsl:param name='p:label'/><xsl:template match='/'><xsl:value-of select='$p:label'/><xsl:apply-templates select='items/item'><xsl:sort select='@n' data-type='number'/></xsl:apply-templates></xsl:template><xsl:template match='item'><xsl:value-of select='.'/></xsl:template></xsl:stylesheet>";
        let values = vec![(
            "urn:parameter".into(),
            "label".into(),
            "'literal & value':".into(),
        )];
        let (result, kind) = transform(
            style,
            "<items><item n='2'>B</item><item n='1'>A</item></items>",
            &values,
            "https://example.test/main.xsl",
            &Resources::default(),
        )
        .unwrap();
        assert_eq!((result.as_str(), kind), ("'literal & value':AB", 2));
    }
    #[test]
    fn xslt_pinned_document_function_is_empty_and_only_xslt1_functions_available() {
        let style = sheet(
            "<xsl:output method='text'/><xsl:template match='/'><xsl:value-of select=\"count(document('file:///etc/passwd'))\"/><xsl:text>:</xsl:text><xsl:value-of select=\"count(document('https://example.test/private.xml'))\"/><xsl:text>:</xsl:text><xsl:value-of select=\"function-available('document')\"/><xsl:text>:</xsl:text><xsl:value-of select=\"function-available('serialize')\"/></xsl:template>",
        );
        assert_eq!(
            transform(
                &style,
                "<source/>",
                &[],
                "https://example.test/main.xsl",
                &Resources::default()
            )
            .unwrap()
            .0,
            "0:0:true:false"
        );
    }
    #[test]
    fn xslt_include_requires_exact_preadmitted_snapshot_and_uses_redirect_base() {
        let main = sheet("<xsl:include href='part.xsl'/>");
        assert!(!validate(
            &main,
            "https://example.test/main.xsl",
            &Resources::default()
        ));
        let resources = Resources(vec![
            (
                "https://example.test/part.xsl".into(),
                "https://example.test/redirect/part.xsl".into(),
                sheet("<xsl:include href='leaf.xsl'/>"),
            ),
            (
                "https://example.test/redirect/leaf.xsl".into(),
                "https://example.test/redirect/leaf.xsl".into(),
                sheet("<xsl:output method='text'/><xsl:template match='/'>included</xsl:template>"),
            ),
        ]);
        assert_eq!(
            transform(
                &main,
                "<r/>",
                &[],
                "https://example.test/main.xsl",
                &resources
            )
            .unwrap()
            .0,
            "included"
        );
        assert!(
            transform(
                &main,
                "<r/>",
                &[],
                "https://elsewhere.test/main.xsl",
                &resources
            )
            .is_none()
        );
    }
    #[test]
    fn xslt_malformed_dtd_and_recursive_stylesheets_fail_without_poisoning_next_transform() {
        let valid = sheet("<xsl:output method='text'/><xsl:template match='/'>next</xsl:template>");
        assert!(!validate(
            "<xsl:stylesheet",
            "https://example.test/",
            &Resources::default()
        ));
        assert!(
            transform(
                &valid,
                "<!DOCTYPE r [<!ENTITY e 'expanded'>]><r>&e;</r>",
                &[],
                "https://example.test/",
                &Resources::default()
            )
            .is_none()
        );
        let recursive = sheet(
            "<xsl:template match='/'><xsl:call-template name='again'/></xsl:template><xsl:template name='again'><xsl:call-template name='again'/></xsl:template>",
        );
        assert!(
            transform(
                &recursive,
                "<r/>",
                &[],
                "https://example.test/",
                &Resources::default()
            )
            .is_none()
        );
        assert_eq!(
            transform(
                &valid,
                "<r/>",
                &[],
                "https://example.test/",
                &Resources::default()
            )
            .unwrap()
            .0,
            "next"
        );
    }
    #[test]
    fn xslt_large_copy_of_hits_native_allocation_budget_then_recovers() {
        let source = format!("<r>{}</r>", "<n>some text</n>".repeat(20_000));
        let explosive = sheet(
            "<xsl:template match='/'><out><xsl:for-each select='r/n'><xsl:copy-of select='/r'/></xsl:for-each></out></xsl:template>",
        );
        assert!(
            transform(
                &explosive,
                &source,
                &[],
                "https://example.test/",
                &Resources::default()
            )
            .is_none()
        );
        let next = sheet("<xsl:output method='text'/><xsl:template match='/'>alive</xsl:template>");
        assert_eq!(
            transform(
                &next,
                "<r/>",
                &[],
                "https://example.test/",
                &Resources::default()
            )
            .unwrap()
            .0,
            "alive"
        );
    }
}
#[test]
fn xslt_dependency_fetch_rejects_cross_origin_and_script_csp_before_transport() {
    for (policy, href) in [
        ("", "https://cross.test/part.xsl"),
        (
            "<meta http-equiv='Content-Security-Policy' content=\"script-src 'none'; connect-src *\">",
            "https://example.test/part.xsl",
        ),
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut page = RustExecutableDocument::load_with_environment(
            &format!("<head>{policy}</head><body/>"),
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(XsltBroker {
                requests: requests.clone(),
                resources: BTreeMap::new(),
            })),
            Some(Url::parse("https://example.test/input.xml").unwrap()),
        )
        .unwrap();
        let source = format!(
            r#"(()=>{{const p=new XSLTProcessor();const s=new DOMParser().parseFromString('<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:include href="{href}"/></xsl:stylesheet>','text/xml');try{{p.importStylesheet(s);return 'not blocked'}}catch(error){{return 'blocked'}}}})()"#
        );
        assert_eq!(page.agent.host.evaluate_string(&source).unwrap(), "blocked");
        assert!(requests.lock().unwrap().is_empty());
    }
}
#[test]
fn xslt_harmless_doctype_text_and_comment_are_not_entity_declarations() {
    let style = r#"<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><!-- <!DOCTYPE example [literal]> --><xsl:output method="text"/><xsl:template match="/"><xsl:text><![CDATA[<!DOCTYPE example [literal]>]]></xsl:text></xsl:template></xsl:stylesheet>"#;
    let result = crate::xslt_process::run(
        oryn_js::platform::XsltRequest {
            style: style.into(),
            source: Some("<r/>".into()),
            parameters: vec![],
            url: "https://example.test/style.xsl".into(),
            resources: vec![],
        },
        || false,
    )
    .unwrap();
    assert_eq!(result.output.unwrap().0, "<!DOCTYPE example [literal]>");
}

#[test]
fn xslt_empty_text_result_is_a_successful_empty_string() {
    let request=oryn_js::platform::XsltRequest {style:"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:output method='text'/><xsl:template match='/'/></xsl:stylesheet>".into(),source:Some("<r/>".into()),parameters:vec![],url:"https://example.test/style.xsl".into(),resources:vec![]};
    let result = crate::xslt_process::run(request, || false).unwrap();
    assert!(result.valid);
    assert_eq!(result.output, Some((String::new(), 2)));
}

#[test]
fn xslt_large_cdata_exceeds_libxml_default_text_limit_within_browser_source_budget() {
    let text = "x".repeat(11_000_000);
    let request=oryn_js::platform::XsltRequest {style:"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:output method='text'/><xsl:template match='/'><xsl:value-of select='string-length(r)'/></xsl:template></xsl:stylesheet>".into(),source:Some(format!("<r><![CDATA[{text}]]></r>")),parameters:vec![],url:"https://example.test/style.xsl".into(),resources:vec![]};
    assert_eq!(
        crate::xslt_process::run(request, || false).unwrap().output,
        Some(("11000000".into(), 2))
    );
}

#[test]
fn xslt_implicit_xml_text_result_has_no_serialization_declaration_or_newline() {
    let request=oryn_js::platform::XsltRequest {style:"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:template match='*'><xsl:value-of select='name(.)'/></xsl:template></xsl:stylesheet>".into(),source:Some("<items/>".into()),parameters:vec![],url:"https://example.test/style.xsl".into(),resources:vec![]};
    assert_eq!(
        crate::xslt_process::run(request, || false).unwrap().output,
        Some(("items".into(), 0))
    );
}

#[test]
fn xslt_empty_xml_result_preserves_success_without_buffer_content() {
    let request=oryn_js::platform::XsltRequest {style:"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:template match='/'/></xsl:stylesheet>".into(),source:Some("<r/>".into()),parameters:vec![],url:"https://example.test/style.xsl".into(),resources:vec![]};
    let result = crate::xslt_process::run(request, || false).unwrap();
    assert!(result.valid);
    assert_eq!(result.output, Some((String::new(), 0)));
}
