//! Native standalone response policy retained with the owning realm. Container
//! edges are captured before loading; redirects cannot rewrite their src origin.
#[cfg(feature = "v8-host")]
use crate::{agent_isolation::DocumentIsolationPolicy, csp::CspPolicySet};
#[cfg(any(feature = "v8-host", test))]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PermissionEdge {
    parent_origin: String,
    child_origin: String,
    headers: Vec<(String, String)>,
    allow: String,
    source: String,
}
#[cfg(any(feature = "v8-host", test))]
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct DocumentPolicy {
    pub(crate) origin: String,
    pub(crate) headers: Vec<(String, String)>,
    edges: Vec<PermissionEdge>,
}
#[cfg(feature = "v8-host")]
#[derive(Clone, Debug)]
pub(crate) struct FramePolicy {
    pub(crate) csp: CspPolicySet,
    pub(crate) document: DocumentPolicy,
}
#[cfg(feature = "v8-host")]
#[derive(Clone, Debug)]
pub(crate) struct BlobResource {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub origin: oryn_js::origin::Origin,
    pub document: DocumentPolicy,
    pub csp: Option<CspPolicySet>,
}
pub(crate) fn retained_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            [
                "permissions-policy",
                "permissions-policy-report-only",
                "cross-origin-embedder-policy",
                "cross-origin-embedder-policy-report-only",
                "cross-origin-opener-policy",
                "origin-agent-cluster",
                "reporting-endpoints",
                "report-to",
            ]
            .iter()
            .any(|field| name.eq_ignore_ascii_case(field))
        })
        .cloned()
        .collect()
}
#[cfg(any(feature = "v8-host", test))]
impl DocumentPolicy {
    #[cfg(feature = "v8-host")]
    pub(crate) fn retained_bytes(&self) -> Result<usize, String> {
        let mut size = crate::retained_size::Size(std::mem::size_of::<Self>());
        size.string(&self.origin)?;
        size.headers(&self.headers)?;
        size.vector(&self.edges)?;
        for edge in &self.edges {
            for value in [
                &edge.parent_origin,
                &edge.child_origin,
                &edge.allow,
                &edge.source,
            ] {
                size.string(value)?;
            }
            size.headers(&edge.headers)?;
        }
        Ok(size.0)
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn snapshot(&self) -> oryn_js::permissions_policy::Snapshot {
        use oryn_js::permissions_policy::{Constraint, Snapshot};
        let fields = |headers: &[(String, String)]| {
            headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut constraints = vec![Constraint::Header {
            origin: self.origin.clone(),
            headers: fields(&self.headers),
        }];
        for edge in &self.edges {
            constraints.push(Constraint::Gate {
                origin: edge.parent_origin.clone(),
                target: edge.parent_origin.clone(),
                headers: fields(&edge.headers),
            });
            constraints.push(Constraint::Gate {
                origin: edge.parent_origin.clone(),
                target: edge.child_origin.clone(),
                headers: fields(&edge.headers),
            });
            constraints.push(Constraint::GateContainer {
                origin: edge.parent_origin.clone(),
                source: edge.source.clone(),
                allow: edge.allow.clone(),
                target: edge.child_origin.clone(),
            });
        }
        Snapshot {
            origin: self.origin.clone(),
            container: false,
            constraints,
        }
    }

    pub(crate) fn root(origin: String, headers: &[(String, String)]) -> Self {
        Self {
            origin,
            headers: retained_headers(headers),
            edges: Vec::new(),
        }
    }
    pub(crate) fn child(
        &self,
        origin: String,
        headers: &[(String, String)],
        allow: String,
        source: String,
    ) -> Result<Self, String> {
        if self.edges.len() >= 256 {
            return Err("standalone Permissions Policy ancestry limit".into());
        }
        let mut edges = self.edges.clone();
        edges.push(PermissionEdge {
            parent_origin: self.origin.clone(),
            child_origin: origin.clone(),
            headers: self.headers.clone(),
            allow,
            source,
        });
        Ok(Self {
            origin,
            headers: retained_headers(headers),
            edges,
        })
    }
    #[cfg(test)]
    pub(crate) fn allows(&self, feature: &str) -> bool {
        use crate::isolation_permissions::{allows_feature, container_allows_feature};
        if !allows_feature(&self.headers, feature, &self.origin, &self.origin) {
            return false;
        }
        self.edges.iter().all(|edge| {
            let declared = edge
                .allow
                .split(';')
                .any(|part| part.split_ascii_whitespace().next() == Some(feature));
            allows_feature(
                &edge.headers,
                feature,
                &edge.parent_origin,
                &edge.parent_origin,
            ) && allows_feature(
                &edge.headers,
                feature,
                &edge.parent_origin,
                &edge.child_origin,
            ) && (feature == "document-domain" && !declared
                || container_allows_feature(
                    &edge.allow,
                    feature,
                    &edge.parent_origin,
                    &edge.source,
                    &edge.child_origin,
                ))
        })
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn own_allows(&self, feature: &str) -> bool {
        crate::isolation_permissions::allows_feature(
            &self.headers,
            feature,
            &self.origin,
            &self.origin,
        )
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn own_report_only_allows(&self, feature: &str) -> bool {
        let mut policy = self.clone();
        policy.edges.clear();
        policy.report_only_allows(feature)
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn edge_allows(
        &self,
        feature: &str,
        child: &str,
        allow: &str,
        source: &str,
    ) -> bool {
        let declared = allow
            .split(';')
            .any(|part| part.split_ascii_whitespace().next() == Some(feature));
        crate::isolation_permissions::allows_feature(&self.headers, feature, &self.origin, child)
            && (feature == "document-domain" && !declared
                || crate::isolation_permissions::container_allows_feature(
                    allow,
                    feature,
                    &self.origin,
                    source,
                    child,
                ))
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn edge_report_only_allows(&self, feature: &str, child: &str) -> bool {
        let fields = self
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy-report-only"))
            .map(|(_, value)| ("permissions-policy".to_owned(), value.clone()))
            .collect::<Vec<_>>();
        !fields.iter().any(|(_, value)| {
            value.split(',').any(|directive| {
                directive
                    .trim()
                    .split_once('=')
                    .is_some_and(|(name, _)| name.trim() == feature)
            })
        }) || crate::isolation_permissions::allows_feature(&fields, feature, &self.origin, child)
    }
    pub(crate) fn report_only_allows(&self, feature: &str) -> bool {
        let allowed = |headers: &[(String, String)], parent: &str, target: &str| {
            let fields = headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy-report-only"))
                .map(|(_, value)| ("permissions-policy".to_owned(), value.clone()))
                .collect::<Vec<_>>();
            !fields.iter().any(|(_, value)| {
                value.split(',').any(|directive| {
                    directive
                        .trim()
                        .split_once('=')
                        .is_some_and(|(name, _)| name.trim() == feature)
                })
            }) || crate::isolation_permissions::allows_feature(&fields, feature, parent, target)
        };
        allowed(&self.headers, &self.origin, &self.origin)
            && self.edges.iter().all(|edge| {
                allowed(&edge.headers, &edge.parent_origin, &edge.parent_origin)
                    && allowed(&edge.headers, &edge.parent_origin, &edge.child_origin)
            })
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn isolation(&self) -> DocumentIsolationPolicy {
        DocumentIsolationPolicy::from_headers(&self.headers)
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn rebase_origin(&mut self, origin: String) {
        self.origin = origin.clone();
        if let Some(edge) = self.edges.last_mut() {
            edge.child_origin = origin;
        }
    }
}

#[cfg(feature = "v8-host")]
impl FramePolicy {
    pub(crate) fn retained_bytes(&self) -> Result<usize, String> {
        self.csp
            .retained_bytes()?
            .checked_add(self.document.retained_bytes()?)
            .ok_or_else(|| "frame policy retained bytes overflow".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_only_denials_do_not_change_enforced_admission() {
        let root = DocumentPolicy::root(
            "https://a.test".into(),
            &[(
                "Permissions-Policy-Report-Only".into(),
                "geolocation=()".into(),
            )],
        );
        assert!(root.allows("geolocation"));
        assert!(!root.report_only_allows("geolocation"));
        let child = root
            .child(
                "https://a.test".into(),
                &[],
                String::new(),
                "https://a.test/child".into(),
            )
            .unwrap();
        assert!(child.allows("geolocation"));
        assert!(!child.report_only_allows("geolocation"));
    }
    #[test]
    fn response_and_container_permissions_survive_redirects_and_nested_edges() {
        let root = DocumentPolicy::root(
            "https://a.test".into(),
            &[(
                "Permissions-Policy".into(),
                "geolocation=(self \"https://b.test\"), document-domain=()".into(),
            )],
        );
        let child = root
            .child(
                "https://b.test".into(),
                &[],
                "geolocation 'src'".into(),
                "https://b.test/frame".into(),
            )
            .unwrap();
        assert!(child.allows("geolocation"));
        assert!(!child.allows("document-domain"));
        let redirected = root
            .child(
                "https://c.test".into(),
                &[],
                "geolocation 'src'".into(),
                "https://b.test/frame".into(),
            )
            .unwrap();
        assert!(!redirected.allows("geolocation"));
        let grandchild = child
            .child(
                "https://b.test".into(),
                &[],
                String::new(),
                "about:blank".into(),
            )
            .unwrap();
        assert!(!grandchild.allows("document-domain"));
        let denied = child
            .child(
                "https://b.test".into(),
                &[("Permissions-Policy".into(), "geolocation=()".into())],
                "geolocation *".into(),
                "https://b.test/frame".into(),
            )
            .unwrap();
        assert!(!denied.allows("geolocation"));
    }
}
