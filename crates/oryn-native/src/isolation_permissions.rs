//! Shared policy parsing and origin matching used by admission and inspection.
pub(crate) use oryn_js::permissions_policy::{
    allows, allows_feature, container_allows, container_allows_feature,
};
