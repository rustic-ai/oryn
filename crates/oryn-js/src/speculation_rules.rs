//! Native speculation rule input, independent of authored object accessors.
use serde_json::Value;
use url::Url;

pub const MAX_RULE_BYTES: usize = 1024 * 1024;
pub const MAX_RULES: usize = 256;
pub const MAX_CANDIDATES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eagerness {
    Immediate,
    Eager,
    Moderate,
    Conservative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetHint {
    SelfTarget,
    Blank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub url: String,
    pub eagerness: Eagerness,
    pub target: Option<TargetHint>,
    pub referrer_policy: Option<String>,
    pub anonymous_cross_origin: bool,
    pub tag: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Capacity,
    InvalidJson,
    InvalidTopLevel,
    InvalidBase,
}

#[derive(Default, Debug)]
pub struct RuleSet {
    pub prerenders: Vec<Candidate>,
    /// Invalid individual rules do not discard valid siblings.
    pub rejected_rules: usize,
}

/// Parse the list-rule contract used by native prerender loading. Unsupported
/// source kinds are rejected explicitly, never converted into eager list rules.
/// Parsing does not authorize a fetch: original-document policy, origin,
/// target hint and candidate eligibility are checked by native admission.
pub fn parse(source: &str, document_base: &str, rules_base: &str) -> Result<RuleSet, ParseError> {
    if source.len() > MAX_RULE_BYTES {
        return Err(ParseError::Capacity);
    }
    let value: Value = serde_json::from_str(source).map_err(|_| ParseError::InvalidJson)?;
    let object = value.as_object().ok_or(ParseError::InvalidTopLevel)?;
    let document_base = Url::parse(document_base).map_err(|_| ParseError::InvalidBase)?;
    let rules_base = Url::parse(rules_base).map_err(|_| ParseError::InvalidBase)?;
    let mut result = RuleSet::default();
    let Some(rules) = object.get("prerender") else {
        return Ok(result);
    };
    let Some(rules) = rules.as_array() else {
        result.rejected_rules = 1;
        return Ok(result);
    };
    if rules.len() > MAX_RULES {
        return Err(ParseError::Capacity);
    }
    for rule in rules {
        match parse_rule(rule, &document_base, &rules_base) {
            Some(candidates) => {
                if result.prerenders.len().saturating_add(candidates.len()) > MAX_CANDIDATES {
                    return Err(ParseError::Capacity);
                }
                result.prerenders.extend(candidates);
            }
            None => result.rejected_rules += 1,
        }
    }
    Ok(result)
}

fn parse_rule(rule: &Value, document_base: &Url, rules_base: &Url) -> Option<Vec<Candidate>> {
    let rule = rule.as_object()?;
    if rule.get("source").and_then(Value::as_str) != Some("list") {
        return None;
    }
    let urls = rule.get("urls")?.as_array()?;
    if urls.len() > MAX_CANDIDATES {
        return None;
    }
    let base = match rule.get("relative_to") {
        None => rules_base,
        Some(Value::String(value)) if value == "ruleset" => rules_base,
        Some(Value::String(value)) if value == "document" => document_base,
        _ => return None,
    };
    let eagerness = match rule.get("eagerness") {
        None => Eagerness::Immediate,
        Some(Value::String(value)) => match value.as_str() {
            "immediate" => Eagerness::Immediate,
            "eager" => Eagerness::Eager,
            "moderate" => Eagerness::Moderate,
            "conservative" => Eagerness::Conservative,
            _ => return None,
        },
        _ => return None,
    };
    let target = match rule.get("target_hint") {
        None => None,
        Some(Value::String(value)) if value == "_self" => Some(TargetHint::SelfTarget),
        Some(Value::String(value)) if value == "_blank" => Some(TargetHint::Blank),
        _ => return None,
    };
    let referrer_policy = match rule.get("referrer_policy") {
        None => None,
        Some(Value::String(value))
            if matches!(
                value.as_str(),
                "no-referrer"
                    | "no-referrer-when-downgrade"
                    | "same-origin"
                    | "origin"
                    | "strict-origin"
                    | "origin-when-cross-origin"
                    | "strict-origin-when-cross-origin"
                    | "unsafe-url"
            ) =>
        {
            Some(value.clone())
        }
        _ => return None,
    };
    let mut anonymous_cross_origin = false;
    if let Some(requirements) = rule.get("requires") {
        for requirement in requirements.as_array()? {
            if requirement.as_str()? != "anonymous-client-ip-when-cross-origin" {
                return None;
            }
            anonymous_cross_origin = true;
        }
    }
    let tag = match rule.get("tag") {
        None => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return None,
    };
    let mut candidates = Vec::new();
    for value in urls {
        let Some(value) = value.as_str() else {
            continue;
        };
        let Ok(url) = base.join(value) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        candidates.push(Candidate {
            url: url.into(),
            eagerness,
            target,
            referrer_policy: referrer_policy.clone(),
            anonymous_cross_origin,
            tag: tag.clone(),
        });
    }
    Some(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_core_pagereveal_helper_defaults_to_immediate_list_prerender() {
        // Pinned pagereveal popup calls startPrerendering(url) with no extras.
        // Its shared helper also defines a separate optional prefetch route;
        // that definition must not alter this caller's admission semantics.
        let rules = parse(r#"{"prerender":[{"source":"list","urls":["https://example.test/popup?uid=token&prerendering"]}]}"#,
            "https://example.test/", "https://example.test/").unwrap();
        assert_eq!(rules.rejected_rules, 0);
        assert_eq!(rules.prerenders.len(), 1);
        assert_eq!(rules.prerenders[0].eagerness, Eagerness::Immediate);
        assert_eq!(rules.prerenders[0].target, None);
        assert_eq!(
            rules.prerenders[0].url,
            "https://example.test/popup?uid=token&prerendering"
        );
    }
    #[test]
    fn native_list_candidates_preserve_policy_and_use_selected_base() {
        let rules = parse(r#"{"prerender":[{"source":"list","urls":["next?x=1#f"],"relative_to":"document","target_hint":"_self","referrer_policy":"no-referrer","requires":["anonymous-client-ip-when-cross-origin"]}]}"#, "https://example.test/document/", "https://example.test/rules/").unwrap();
        assert_eq!(
            rules.prerenders[0].url,
            "https://example.test/document/next?x=1#f"
        );
        assert_eq!(rules.prerenders[0].target, Some(TargetHint::SelfTarget));
        assert!(rules.prerenders[0].anonymous_cross_origin);
        assert_eq!(
            rules.prerenders[0].referrer_policy.as_deref(),
            Some("no-referrer")
        );
    }
    #[test]
    fn malformed_rules_do_not_invent_candidates_or_discard_valid_siblings() {
        assert_eq!(
            parse("[]", "https://a.test/", "https://a.test/").unwrap_err(),
            ParseError::InvalidTopLevel
        );
        let rules = parse(r#"{"prerender":[{"source":"list","urls":["/bad"],"requires":["unknown"]},{"source":"document","where":{}},{"source":"list","urls":[9,"javascript:alert(1)","/valid"]}]}"#, "https://a.test/", "https://a.test/").unwrap();
        assert_eq!(rules.rejected_rules, 2);
        assert_eq!(rules.prerenders.len(), 1);
        assert_eq!(rules.prerenders[0].url, "https://a.test/valid");
    }
}
