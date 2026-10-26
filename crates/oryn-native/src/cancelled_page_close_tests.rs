//! A dropped close future must not claim physical completion or lose retry custody.
use super::*;

#[tokio::test]
async fn cancelled_close_waiter_preserves_exact_page_custody_for_retry() {
    for crashed in [false, true] {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let sibling = context.new_page();
        let id = page.page_id();
        let sender = page.sender.as_ref().unwrap().clone();
        let physical = context.inner.group.pages.lock().unwrap()[&id]
            .physical_close
            .clone();
        let held = physical.lock().await;
        if crashed {
            context
                .inner
                .group
                .crash(id, "cancelled close regression".into());
        }
        let mut closing = Box::pin(context.close_page(id));
        let polled = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(closing.as_mut(), cx))
        })
        .await;
        assert!(matches!(polled, std::task::Poll::Pending));
        drop(closing);
        assert!(!context.inner.group.pages.lock().unwrap()[&id].physical_close_complete);
        assert!(!sender.is_closed());
        assert!(context.inner.group.handle(sibling.page_id()).is_some());
        assert!(!sibling.sender.as_ref().unwrap().is_closed());
        drop(held);
        context.close_page(id).await.unwrap();
        assert!(sender.is_closed());
        assert!(context.inner.group.pages.lock().unwrap()[&id].physical_close_complete);
        context.close_page(id).await.unwrap();
        assert!(!sibling.sender.as_ref().unwrap().is_closed());
        sibling.close().await.unwrap();
        context.retire_workers().await.unwrap();
    }
}
