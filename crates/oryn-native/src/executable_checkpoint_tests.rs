//! Fault injection at the last phase of the real executable checkpoint. The
//! injected callbacks model work published after an earlier realm/drain phase.
use super::*;
use std::cell::{Cell, RefCell};

type Hook = Box<dyn FnMut(&mut RustExecutableDocument, usize)>;
thread_local! {
    static AFTER_PASS: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

struct HookGuard(Option<Hook>);
impl HookGuard {
    fn install(hook: impl FnMut(&mut RustExecutableDocument, usize) + 'static) -> Self {
        Self(AFTER_PASS.with(|slot| slot.replace(Some(Box::new(hook)))))
    }
}
impl Drop for HookGuard {
    fn drop(&mut self) {
        AFTER_PASS.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}
pub(super) fn after_pass(page: &mut RustExecutableDocument, pass: usize) {
    // A nested checkpoint cannot recursively invoke this test callback.
    let hook = AFTER_PASS.with(|slot| slot.borrow_mut().take());
    if let Some(mut hook) = hook {
        hook(page, pass);
        AFTER_PASS.with(|slot| {
            slot.replace(Some(hook));
        });
    }
}

fn quiet_page() -> RustExecutableDocument {
    let mut page = RustExecutableDocument::load("<body id=body></body>").unwrap();
    // The outer page owner normally consumes this root history publication.
    page.agent.host.take_navigation_history_update();
    // Initial page reveal belongs to the outer rendering lifecycle. Merely
    // checkpointing leaves its 16 ms deadline pending, so an idle assertion
    // would depend on machine speed once that deadline becomes runnable.
    page.run_animation_frame_phase().unwrap();
    page.checkpoint().unwrap();
    assert!(!page.has_runnable_page_work());
    page
}
fn checkpoint_passes(page: &mut RustExecutableDocument) -> usize {
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels(|| page.checkpoint());
    result.unwrap();
    labels
        .into_iter()
        .filter(|label| *label == "checkpoint_realm_retirement")
        .count()
}

#[test]
fn outer_history_publication_does_not_keep_checkpoint_spinning() {
    let mut page = quiet_page();
    page.agent
        .host
        .execute("history.replaceState({saved: 1}, '')")
        .unwrap();
    assert!(!page.has_runnable_page_work());
    assert_eq!(checkpoint_passes(&mut page), 1);
    // The outer owner must still receive the publication after we yield.
    assert!(page.take_navigation_history_update().is_some());
    assert_eq!(checkpoint_passes(&mut page), 1);
}

#[test]
fn intercepted_navigation_waits_without_spinning_and_resumes_on_settlement() {
    let mut page = RustExecutableDocument::load_with_environment(
        "<body></body>",
        &BTreeMap::new(),
        None,
        Some(Url::parse("https://example.test/start").unwrap()),
    )
    .unwrap();
    page.run_animation_frame_phase().unwrap();
    page.agent
        .host
        .execute(
            r#"
        globalThis.navigationFinished = false;
        navigation.onnavigate = event => event.intercept({
            handler: () => new Promise(resolve => globalThis.finishNavigation = resolve)
        });
        navigation.navigate('#pending').finished.then(() => navigationFinished = true);
    "#,
        )
        .unwrap();
    page.checkpoint().unwrap();
    assert_eq!(
        page.evaluate("typeof finishNavigation").unwrap(),
        "function"
    );
    page.agent.host.take_navigation_history_update();
    page.run_animation_frame_phase().unwrap();
    assert_eq!(checkpoint_passes(&mut page), 1);
    assert_eq!(
        page.evaluate("String(navigationFinished)").unwrap(),
        "false"
    );
    page.agent.host.execute("finishNavigation()").unwrap();
    page.checkpoint().unwrap();
    assert_eq!(page.evaluate("String(navigationFinished)").unwrap(), "true");
}

#[test]
fn late_child_history_publication_still_reaches_the_frame_owner() {
    let mut page = quiet_page();
    page.agent
        .host
        .execute("document.body.append(document.createElement('iframe'))")
        .unwrap();
    page.checkpoint().unwrap();
    let child = *page.frame_realms.keys().next().unwrap();
    let _hook = HookGuard::install(move |page, pass| {
        if pass == 0 {
            page.agent.host.activate_realm(child).unwrap();
            page.agent
                .host
                .execute("history.replaceState({saved: 1}, '')")
                .unwrap();
        }
    });
    assert!(checkpoint_passes(&mut page) >= 2);
    page.agent.host.activate_realm(child).unwrap();
    assert!(page.agent.host.take_navigation_history_update().is_none());
}

#[test]
fn idle_checkpoint_runs_once_and_next_invocation_observes_external_work() {
    let mut page = quiet_page();
    assert_eq!(checkpoint_passes(&mut page), 1);
    let before = page.agent.host.executable_checkpoint_witness();
    page.agent.host.execute("globalThis.later=0;document.body.append(document.createElement('iframe'));queueMicrotask(()=>later=1)").unwrap();
    assert_ne!(page.agent.host.executable_checkpoint_witness(), before);
    assert!(checkpoint_passes(&mut page) >= 1);
    assert_eq!(
        page.agent
            .host
            .evaluate_string(
                "String(later)+':'+String(document.querySelector('iframe').contentDocument!==null)"
            )
            .unwrap(),
        "1:true"
    );
    assert_eq!(page.agent.host.active_realm(), page.top_realm);
}

#[test]
fn mutation_only_late_phase_reaches_observer_and_later_script_drain() {
    let mut page = quiet_page();
    page.agent.host.execute(r#"
        globalThis.laterScript=0;
        new MutationObserver(()=>{
            if(document.body.hasAttribute('data-late')&&!laterScript){
                const s=document.createElement('script');s.textContent='laterScript=1';document.body.append(s);
            }
        }).observe(document.body,{attributes:true});
    "#).unwrap();
    let body = page.agent.dom.borrow().find_by_id("body").unwrap();
    let _hook = HookGuard::install(move |page, pass| {
        if pass == 0 {
            page.agent
                .dom
                .borrow_mut()
                .set_attribute(body, "data-late", "yes")
                .unwrap();
        }
    });
    assert!(
        checkpoint_passes(&mut page) >= 2,
        "a late mutation must trigger another full pass"
    );
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(laterScript)")
            .unwrap(),
        "1"
    );
}

#[test]
fn later_child_realm_schedules_work_in_previously_visited_parent() {
    let mut page = quiet_page();
    page.agent
        .host
        .execute("globalThis.fromChild=0;document.body.append(document.createElement('iframe'))")
        .unwrap();
    page.checkpoint().unwrap();
    let child = *page.frame_realms.keys().next().unwrap();
    let _hook = HookGuard::install(move |page, pass| {
        if pass == 0 {
            page.agent.host.activate_realm(child).unwrap();
            page.agent.host.execute("const s=parent.document.createElement('script');s.textContent='fromChild=7';parent.document.body.append(s)").unwrap();
        }
    });
    assert!(checkpoint_passes(&mut page) >= 2);
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(fromChild)")
            .unwrap(),
        "7"
    );
    assert_eq!(page.agent.host.active_realm(), page.top_realm);
}

#[test]
fn active_mutations_keep_eight_pass_cap_and_do_not_cache_idle() {
    let mut page = quiet_page();
    let body = page.agent.dom.borrow().find_by_id("body").unwrap();
    let observed = Rc::new(Cell::new(0));
    let count = observed.clone();
    {
        let _hook = HookGuard::install(move |page, pass| {
            count.set(count.get() + 1);
            page.agent
                .dom
                .borrow_mut()
                .set_attribute(body, "data-pass", (pass + 1).to_string())
                .unwrap();
        });
        assert_eq!(checkpoint_passes(&mut page), 8);
    }
    assert_eq!(observed.get(), 8);
    // The previous cap is not a cached claim about the following invocation.
    let next = checkpoint_passes(&mut page);
    assert!((1..=8).contains(&next));
    assert_eq!(observed.get(), 8, "the mutation hook must be retired");
    assert_eq!(checkpoint_passes(&mut page), 1);
}
