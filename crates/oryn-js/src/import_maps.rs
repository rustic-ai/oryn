//! Native per-environment import-map resolution. No page-authored JS is consulted.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use url::Url;

type Specifiers = BTreeMap<String, Option<Url>>;

#[derive(Clone, Default)]
pub struct ImportMap {
    imports: Specifiers,
    scopes: BTreeMap<String, Specifiers>,
    integrity: BTreeMap<String, String>,
    resolved: BTreeSet<(String, String, bool)>,
    resolved_bytes: usize,
}

impl ImportMap {
    pub fn retained_bytes(&self) -> usize {
        fn entries(map: &Specifiers) -> usize {
            map.iter().fold(0usize, |size, (key, value)| {
                size.saturating_add(96)
                    .saturating_add(key.len())
                    .saturating_add(value.as_ref().map_or(0, |url| url.as_str().len()))
            })
        }
        entries(&self.imports)
            .saturating_add(self.scopes.iter().fold(0usize, |size, (scope, map)| {
                size.saturating_add(96)
                    .saturating_add(scope.len())
                    .saturating_add(entries(map))
            }))
            .saturating_add(self.integrity.iter().fold(0usize, |size, (key, value)| {
                size.saturating_add(96)
                    .saturating_add(key.len())
                    .saturating_add(value.len())
            }))
            .saturating_add(
                self.resolved
                    .iter()
                    .fold(0usize, |size, (base, specifier, _)| {
                        size.saturating_add(96)
                            .saturating_add(base.len())
                            .saturating_add(specifier.len())
                    }),
            )
    }
}

fn url_like(specifier: &str, base: &Url) -> Option<Url> {
    if specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../") {
        base.join(specifier).ok()
    } else {
        Url::parse(specifier).ok()
    }
}

fn prefix_allowed(url: Option<&Url>) -> bool {
    url.is_none_or(|url| {
        matches!(
            url.scheme(),
            "ftp" | "file" | "http" | "https" | "ws" | "wss"
        )
    })
}

fn matches_key(key: &str, specifier: &str, prefix: bool) -> bool {
    key == specifier || (prefix && key.ends_with('/') && specifier.starts_with(key))
}

fn matches_scope(scope: &str, base: &str) -> bool {
    scope == base || (scope.ends_with('/') && base.starts_with(scope))
}

fn object(value: &Value) -> Result<&Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| "import map member must be an object".into())
}

fn normalize(entries: &Value, base: &Url) -> Result<Specifiers, String> {
    let mut result = Specifiers::new();
    for (key, value) in object(entries)? {
        if key.is_empty() {
            continue;
        }
        let key = url_like(key, base)
            .map(|url| url.to_string())
            .unwrap_or_else(|| key.clone());
        let target = value
            .as_str()
            .and_then(|value| url_like(value, base))
            .filter(|url| !key.ends_with('/') || url.as_str().ends_with('/'));
        result.insert(key, target);
    }
    Ok(result)
}

impl ImportMap {
    pub fn register(&mut self, source: &str, base: &Url) -> Result<(), String> {
        if source.len() > 1024 * 1024 {
            return Err("import map exceeds 1 MiB limit".into());
        }
        let value: Value = serde_json::from_str(source).map_err(|error| error.to_string())?;
        let parsed = object(&value)?;
        // Parse and merge into a candidate: invalid structure or limits do not
        // leave a partially installed map.
        let mut next = self.clone();
        if let Some(imports) = parsed.get("imports") {
            for (key, target) in normalize(imports, base)? {
                if !self
                    .resolved
                    .iter()
                    .any(|(_, specifier, prefix)| matches_key(&key, specifier, *prefix))
                {
                    next.imports.entry(key).or_insert(target);
                }
            }
        }
        if let Some(scopes) = parsed.get("scopes") {
            for (scope, entries) in object(scopes)? {
                // Invalid scope maps are errors even when their URL is invalid.
                let entries = normalize(entries, base)?;
                let Ok(scope) = base.join(scope) else {
                    continue;
                };
                let scope = scope.to_string();
                for (key, target) in entries {
                    if !self.resolved.iter().any(|(referrer, specifier, prefix)| {
                        matches_scope(&scope, referrer) && matches_key(&key, specifier, *prefix)
                    }) {
                        next.scopes
                            .entry(scope.clone())
                            .or_default()
                            .entry(key)
                            .or_insert(target);
                    }
                }
            }
        }
        if let Some(integrity) = parsed.get("integrity") {
            for (key, value) in object(integrity)? {
                if let (Ok(url), Some(value)) = (base.join(key), value.as_str()) {
                    next.integrity
                        .entry(url.to_string())
                        .or_insert_with(|| value.to_owned());
                }
            }
        }
        let entries = next.imports.len()
            + next.scopes.values().map(BTreeMap::len).sum::<usize>()
            + next.integrity.len();
        let bytes = next
            .imports
            .iter()
            .chain(next.scopes.values().flat_map(|entries| entries.iter()))
            .map(|(key, target)| key.len() + target.as_ref().map_or(0, |url| url.as_str().len()))
            .sum::<usize>()
            + next.scopes.keys().map(String::len).sum::<usize>()
            + next
                .integrity
                .iter()
                .map(|(key, value)| key.len() + value.len())
                .sum::<usize>();
        if entries > 4096 || bytes > 1024 * 1024 {
            return Err("merged import map exceeds resource limit".into());
        }
        *self = next;
        Ok(())
    }

    pub fn resolve(&self, base: &Url, specifier: &str) -> Result<Option<String>, String> {
        let as_url = url_like(specifier, base);
        let normalized = as_url.as_ref().map(|url| url.as_str()).unwrap_or(specifier);
        let mut result = None;
        for (scope, entries) in self.scopes.iter().rev() {
            if matches_scope(scope, base.as_str()) {
                result = resolve_match(entries, normalized, prefix_allowed(as_url.as_ref()))?;
                if result.is_some() {
                    break;
                }
            }
        }
        if result.is_none() {
            result = resolve_match(&self.imports, normalized, prefix_allowed(as_url.as_ref()))?;
        }
        let result = result.or(as_url);
        Ok(result.map(|url| url.to_string()))
    }

    /// Integrity metadata is keyed by the resolved request URL. Fetch must
    /// snapshot it before redirects and verify representation bytes before decode.
    pub fn integrity_for(&self, url: &str) -> Option<&str> {
        self.integrity
            .get(url)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    pub fn record_resolution(&mut self, base: &Url, specifier: &str) -> Result<(), String> {
        let url = url_like(specifier, base);
        let record = (
            base.to_string(),
            url.as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| specifier.into()),
            prefix_allowed(url.as_ref()),
        );
        if !self.resolved.contains(&record) {
            let bytes = record.0.len() + record.1.len();
            if self.resolved.len() >= 8192 || self.resolved_bytes + bytes > 1024 * 1024 {
                return Err("resolved module set exceeds resource limit".into());
            }
            self.resolved_bytes += bytes;
        }
        self.resolved.insert(record);
        Ok(())
    }
}

fn resolve_match(
    entries: &Specifiers,
    specifier: &str,
    prefix: bool,
) -> Result<Option<Url>, String> {
    for (key, target) in entries.iter().rev() {
        if !matches_key(key, specifier, prefix) {
            continue;
        }
        let target = target
            .as_ref()
            .ok_or_else(|| format!("module {specifier:?} blocked by null import-map entry"))?;
        if key == specifier {
            return Ok(Some(target.clone()));
        }
        let resolved = target
            .join(&specifier[key.len()..])
            .map_err(|error| error.to_string())?;
        if !resolved.as_str().starts_with(target.as_str()) {
            return Err("module specifier backtracks above mapped prefix".into());
        }
        return Ok(Some(resolved));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_prefix_scopes_and_null_are_native() {
        let base = Url::parse("https://test.example/app/index.html").unwrap();
        let mut map = ImportMap::default();
        map.register(r##"{"imports":{"#entry":"./entry.js","pkg/":"/vendor/","blocked":null},"scopes":{"./deep/":{"#entry":"./deep.js"},"./deep/nested/":{"other":"./other.js"}}}"##, &base).unwrap();
        assert_eq!(
            map.resolve(&base, "#entry").unwrap().unwrap(),
            "https://test.example/app/entry.js"
        );
        assert_eq!(
            map.resolve(&base, "pkg/sub.js").unwrap().unwrap(),
            "https://test.example/vendor/sub.js"
        );
        assert_eq!(
            map.resolve(&base.join("deep/nested/main.js").unwrap(), "#entry")
                .unwrap()
                .unwrap(),
            "https://test.example/app/deep.js"
        );
        assert!(map.resolve(&base, "blocked").is_err());
        assert!(map.resolve(&base, "pkg/../escape.js").is_err());
        assert!(map.resolve(&base, "pkg/%2e%2e/escape.js").is_err());
        assert_eq!(map.resolve(&base, "unmapped").unwrap(), None);
    }

    #[test]
    fn merge_cannot_change_existing_or_resolved_imports() {
        let base = Url::parse("https://test.example/app/main.js").unwrap();
        let mut map = ImportMap::default();
        map.register(r#"{"imports":{"a":"./a.js"}}"#, &base)
            .unwrap();
        map.record_resolution(&base, "./existing.js").unwrap();
        map.register(r#"{"imports":{"a":"./other.js","./existing.js":"./wrong.js","b":"./b.js"},"scopes":{"./":{"./existing.js":"./bad.js"}}}"#, &base).unwrap();
        assert!(map.resolve(&base, "a").unwrap().unwrap().ends_with("/a.js"));
        assert!(
            map.resolve(&base, "./existing.js")
                .unwrap()
                .unwrap()
                .ends_with("/existing.js")
        );
        assert!(map.resolve(&base, "b").unwrap().unwrap().ends_with("/b.js"));
    }

    #[test]
    fn invalid_structure_is_atomic_and_integrity_is_retained_for_fetch() {
        let base = Url::parse("https://test.example/").unwrap();
        let mut map = ImportMap::default();
        assert!(
            map.register(r#"{"imports":{"a":"./a.js"},"scopes":[]}"#, &base)
                .is_err()
        );
        assert_eq!(map.resolve(&base, "a").unwrap(), None);
        map.register(r#"{"imports":{"a":"./a.js","bad/":"./not-prefix","bare":"bare"},"integrity":{"./a.js":"sha256-required"}}"#, &base).unwrap();
        for name in ["a", "./a.js"] {
            assert_eq!(
                map.resolve(&base, name).unwrap(),
                Some("https://test.example/a.js".into())
            );
        }
        assert_eq!(
            map.integrity_for("https://test.example/a.js"),
            Some("sha256-required")
        );
        for name in ["bad/file.js", "bare"] {
            assert!(map.resolve(&base, name).is_err());
        }
    }

    #[test]
    fn import_map_resource_limits_are_atomic_and_bounded() {
        let base = Url::parse("https://test.example/").unwrap();
        let mut map = ImportMap::default();
        map.register(r#"{"imports":{"kept":"./kept.js"}}"#, &base)
            .unwrap();
        let imports: Map<String, Value> = (0..4096)
            .map(|i| (format!("key{i}"), Value::String(format!("./{i}.js"))))
            .collect();
        assert!(
            map.register(&serde_json::json!({"imports":imports}).to_string(), &base)
                .is_err()
        );
        assert_eq!(map.imports.len(), 1);
        assert!(map.resolve(&base, "kept").unwrap().is_some());
        assert!(map.register(&" ".repeat(1024 * 1024 + 1), &base).is_err());
        let specifier = "a".repeat(2048);
        assert!((0..1024).any(|i| {
            map.record_resolution(&base, &format!("{specifier}{i}"))
                .is_err()
        }));
        assert!(map.resolved_bytes <= 1024 * 1024);
    }
}
