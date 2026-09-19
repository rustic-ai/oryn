//! Parent-owned browsing-context groups. Renderer Window wrappers retain an
//! opaque group token, so a COOP switch cannot silently retarget an old proxy.
use oryn_common::v3::PageId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupToken([u8; 16]);
impl std::fmt::Debug for GroupToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupToken([native identity])")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coop {
    UnsafeNone,
    SameOrigin,
    SameOriginAllowPopups,
    NoopenerAllowPopups,
    RestrictProperties,
}
impl Coop {
    pub(crate) fn from_headers(headers: &[(String, String)]) -> Self {
        let mut fields = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cross-origin-opener-policy"));
        let Some((_, value)) = fields.next() else {
            return Self::UnsafeNone;
        };
        if fields.next().is_some() {
            return Self::UnsafeNone;
        }
        let Some(token) = crate::agent_isolation::structured_token(value) else {
            return Self::UnsafeNone;
        };
        match token {
            "same-origin" => Self::SameOrigin,
            "same-origin-allow-popups" => Self::SameOriginAllowPopups,
            "noopener-allow-popups" => Self::NoopenerAllowPopups,
            "restrict-properties" => Self::RestrictProperties,
            _ => Self::UnsafeNone,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct DocumentGroup {
    pub token: GroupToken,
    pub policy: Coop,
    pub origin: String,
    pub initial: bool,
}
#[derive(Default, Debug)]
pub(crate) struct BrowsingGroups {
    documents: BTreeMap<PageId, DocumentGroup>,
}
fn fresh() -> Result<GroupToken, String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    if bytes == [0; 16] {
        return Err("group entropy unavailable".into());
    }
    Ok(GroupToken(bytes))
}
impl BrowsingGroups {
    /// A prerender owns a distinct browsing context before activation. Its
    /// history entry carries this exact group into the visible traversable.
    pub(crate) fn independent_document(
        origin: String,
        policy: Coop,
    ) -> Result<DocumentGroup, String> {
        Ok(DocumentGroup {
            token: fresh()?,
            policy,
            origin,
            initial: false,
        })
    }
    pub(crate) fn register(
        &mut self,
        page: PageId,
        opener: Option<PageId>,
        origin: String,
    ) -> Result<(), String> {
        if self.documents.contains_key(&page) {
            return Err("browsing context already registered".into());
        }
        let inherited = opener.and_then(|opener| self.documents.get(&opener));
        let (token, policy) = match inherited {
            Some(parent) => (parent.token, parent.policy),
            None => (fresh()?, Coop::UnsafeNone),
        };
        self.documents.insert(
            page,
            DocumentGroup {
                token,
                policy,
                origin,
                initial: true,
            },
        );
        Ok(())
    }
    /// Return true when the committed response requires the old opener graph
    /// to be severed. The caller publishes this atomically with the descriptor.
    pub(crate) fn commit(
        &mut self,
        page: PageId,
        origin: String,
        policy: Coop,
    ) -> Result<bool, String> {
        let previous = self
            .documents
            .get(&page)
            .ok_or("browsing context missing")?;
        let same_origin = previous.origin == origin;
        let matching = (previous.policy == Coop::UnsafeNone && policy == Coop::UnsafeNone)
            || (previous.policy == policy && same_origin && policy != Coop::NoopenerAllowPopups)
            || (previous.initial
                && previous.policy == Coop::SameOriginAllowPopups
                && policy == Coop::UnsafeNone);
        let switch = !matching;
        let token = if switch { fresh()? } else { previous.token };
        self.documents.insert(
            page,
            DocumentGroup {
                token,
                policy,
                origin,
                initial: false,
            },
        );
        Ok(switch)
    }
    pub(crate) fn token(&self, page: PageId) -> Option<GroupToken> {
        self.documents.get(&page).map(|document| document.token)
    }
    pub(crate) fn snapshot(&self, page: PageId) -> Option<DocumentGroup> {
        self.documents.get(&page).cloned()
    }
    pub(crate) fn restore(&mut self, page: PageId, snapshot: DocumentGroup) -> Result<(), String> {
        if !self.documents.contains_key(&page) {
            return Err("restored browsing context missing".into());
        }
        self.documents.insert(page, snapshot);
        Ok(())
    }
    pub(crate) fn related(
        &self,
        source: PageId,
        target: PageId,
        expected: Option<GroupToken>,
    ) -> bool {
        let (Some(source), Some(target)) =
            (self.documents.get(&source), self.documents.get(&target))
        else {
            return false;
        };
        source.token == target.token && expected.is_none_or(|expected| expected == source.token)
    }
    pub(crate) fn retire(&mut self, page: PageId) {
        self.documents.remove(&page);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_inheritance_switch_and_stale_proxy_do_not_retarget() {
        let mut groups = BrowsingGroups::default();
        let opener = PageId(1);
        let popup = PageId(2);
        groups
            .register(opener, None, "native-origin-a".into())
            .unwrap();
        groups
            .commit(opener, "native-origin-a".into(), Coop::SameOrigin)
            .unwrap();
        groups
            .register(popup, Some(opener), "native-origin-a".into())
            .unwrap();
        let token = groups.token(opener).unwrap();
        assert!(groups.related(opener, popup, Some(token)));
        assert!(
            groups
                .commit(popup, "native-origin-b".into(), Coop::UnsafeNone)
                .unwrap()
        );
        assert!(!groups.related(opener, popup, Some(token)));
        groups
            .commit(popup, "native-origin-a".into(), Coop::SameOrigin)
            .unwrap();
        assert!(
            !groups.related(opener, popup, Some(token)),
            "returning to an origin cannot resurrect old group identity"
        );
    }
    #[test]
    fn allow_popups_exception_does_not_apply_to_later_navigations() {
        let mut groups = BrowsingGroups::default();
        groups.register(PageId(1), None, "a".into()).unwrap();
        groups
            .commit(PageId(1), "a".into(), Coop::SameOriginAllowPopups)
            .unwrap();
        groups
            .register(PageId(2), Some(PageId(1)), "a".into())
            .unwrap();
        assert!(
            !groups
                .commit(PageId(2), "b".into(), Coop::UnsafeNone)
                .unwrap()
        );
        assert!(groups.related(PageId(1), PageId(2), None));
        assert!(
            groups
                .commit(PageId(2), "b".into(), Coop::SameOriginAllowPopups)
                .unwrap()
        );
        let snapshot = groups.snapshot(PageId(2)).unwrap();
        groups
            .commit(PageId(2), "c".into(), Coop::UnsafeNone)
            .unwrap();
        groups.restore(PageId(2), snapshot.clone()).unwrap();
        assert_eq!(groups.token(PageId(2)), Some(snapshot.token));
        groups.retire(PageId(2));
        assert!(!groups.related(PageId(1), PageId(2), None));
    }
}
