//! WebDriver timeout configuration shared by session creation and updates.
use super::CommandError;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub script: Option<u64>,
    pub page_load: Option<u64>,
    pub implicit: Option<u64>,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            script: Some(30_000),
            page_load: Some(300_000),
            implicit: Some(0),
        }
    }
}

impl Timeouts {
    /// Produce a new configuration only after every recognized field validates.
    /// Null is distinct from both zero and an omitted field.
    pub fn updated(self, value: &Value) -> Result<Self, CommandError> {
        let fields = value
            .as_object()
            .ok_or_else(|| CommandError::invalid("timeouts must be an object"))?;
        let mut next = self;
        for (key, value) in fields {
            let target = match key.as_str() {
                "script" => &mut next.script,
                "pageLoad" => &mut next.page_load,
                "implicit" => &mut next.implicit,
                _ => continue,
            };
            *target = if value.is_null() {
                None
            } else {
                let number = value
                    .as_f64()
                    .filter(|n| {
                        n.is_finite() && *n >= 0. && *n <= 9_007_199_254_740_991. && n.fract() == 0.
                    })
                    .ok_or_else(|| CommandError::invalid(format!("invalid {key} timeout")))?;
                Some(number as u64)
            };
        }
        Ok(next)
    }

    pub fn to_json(self) -> Value {
        json!({"script":self.script,"pageLoad":self.page_load,"implicit":self.implicit})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_partial_updates_preserve_distinct_timeout_values() {
        let original = Timeouts::default();
        assert_eq!(
            original.to_json(),
            json!({"script":30000,"pageLoad":300000,"implicit":0})
        );
        let next = original
            .updated(&json!({"implicit":2.0,"unrecognized":{"anything":true}}))
            .unwrap();
        assert_eq!(
            next,
            Timeouts {
                implicit: Some(2),
                ..original
            }
        );
        assert_eq!(original.updated(&json!({})).unwrap(), original);
        assert_eq!(
            original.updated(&json!({"unrecognized":null})).unwrap(),
            original
        );
    }
    #[test]
    fn every_timeout_accepts_null_zero_and_largest_safe_integer() {
        for key in ["script", "pageLoad", "implicit"] {
            for value in [
                Value::Null,
                json!(0),
                json!(2.0),
                json!(9_007_199_254_740_991u64),
            ] {
                let mut request = serde_json::Map::new();
                request.insert(key.into(), value.clone());
                let result = Timeouts::default()
                    .updated(&Value::Object(request))
                    .unwrap()
                    .to_json();
                if value.is_null() {
                    assert!(result[key].is_null());
                } else {
                    assert_eq!(result[key].as_f64(), value.as_f64());
                }
            }
        }
    }
    #[test]
    fn invalid_updates_are_atomic_and_reject_boolean_and_fractional_numbers() {
        let original = Timeouts {
            script: Some(7),
            page_load: None,
            implicit: Some(9),
        };
        for invalid in [
            json!(-1),
            json!(2.5),
            json!(9_007_199_254_740_992u64),
            json!(false),
            json!("10"),
            json!([]),
            json!({}),
        ] {
            let request = json!({"implicit":100,"pageLoad":invalid,"script":null});
            assert_eq!(
                original.updated(&request).unwrap_err().code,
                "invalid argument"
            );
            assert_eq!(
                original,
                Timeouts {
                    script: Some(7),
                    page_load: None,
                    implicit: Some(9)
                }
            );
        }
        for invalid in [Value::Null, json!(1), json!([]), json!(false)] {
            assert!(original.updated(&invalid).is_err());
        }
    }
}
