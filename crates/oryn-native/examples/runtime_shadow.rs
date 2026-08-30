use oryn_common::v3::SemanticAction;
use oryn_native::{Browser, NativeAction};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let browser = Browser::in_process_probe();
    let context = browser.new_context();
    let mut results = Vec::new();

    let page = context.new_page();
    page.load_html("https://shadow.test/tree", "<main id=app></main>")
        .await?;
    page.evaluate("const item=document.createElement('p');item.id='message';item.textContent='ready';document.getElementById('app').appendChild(item)")
        .await?;
    results.push(case(
        "tree-mutation",
        page.evaluate("document.getElementById('message').textContent")
            .await?
            == "ready",
    ));

    let page = context.new_page();
    page.load_html(
        "https://shadow.test/event",
        "<main><button id=go>Run</button></main>",
    )
    .await?;
    page.evaluate("document.getElementById('go').addEventListener('click',event=>event.currentTarget.textContent='Done')")
        .await?;
    let target = page.resolve_selector("#go", SemanticAction::Click).await?;
    page.execute(NativeAction {
        target,
        action: SemanticAction::Click,
        value: None,
    })
    .await?;
    results.push(case(
        "event-mutation",
        page.evaluate("document.getElementById('go').textContent")
            .await?
            == "Done",
    ));

    let page = context.new_page();
    page.load_html(
        "https://shadow.test/fragment",
        "<section id=target></section>",
    )
    .await?;
    page.evaluate(
        "document.getElementById('target').innerHTML='<strong id=created>native</strong>'",
    )
    .await?;
    results.push(case(
        "contextual-fragment",
        page.evaluate("document.getElementById('created').textContent")
            .await?
            == "native",
    ));

    let page = context.new_page();
    page.load_html(
        "https://shadow.test/selector",
        "<main id=app><section class=panel><button class=primary data-key=7>Go</button></section></main>",
    )
    .await?;
    page.evaluate("document.querySelector('#app .panel button.primary[data-key=\"7\"]').textContent='Matched'")
        .await?;
    results.push(case(
        "compound-selector",
        page.evaluate("document.querySelector('button').textContent")
            .await?
            == "Matched",
    ));

    println!("{}", serde_json::to_string_pretty(&results)?);
    if results.iter().all(|result| result["passed"] == true) {
        Ok(())
    } else {
        Err("runtime shadow fixture failed".into())
    }
}

fn case(id: &str, passed: bool) -> serde_json::Value {
    serde_json::json!({ "id": id, "passed": passed })
}
