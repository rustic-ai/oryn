//! Ordered content attributes. Mutation is restricted to the native DOM crate.
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AttributeKey {
    pub namespace: Option<String>,
    pub local_name: String,
}

impl AttributeKey {
    pub fn new(namespace: Option<&str>, local_name: &str) -> Self {
        Self {
            namespace: namespace
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            local_name: local_name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAttribute {
    pub namespace: Option<String>,
    pub local_name: String,
    pub name: String,
    pub prefix: Option<String>,
    pub value: String,
}

impl ContentAttribute {
    pub(crate) fn key(&self) -> AttributeKey {
        AttributeKey::new(self.namespace.as_deref(), &self.local_name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeList(Vec<Arc<ContentAttribute>>);

impl AttributeList {
    pub fn records(&self) -> impl Iterator<Item = &ContentAttribute> {
        self.0.iter().map(AsRef::as_ref)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter().map(|entry| (&entry.name, &entry.value))
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.iter().map(|entry| &entry.name)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, name: &str) -> Option<&String> {
        self.named(name).map(|entry| &entry.value)
    }
    /// HTML reflection and algorithms address null-namespace attributes,
    /// unlike the qualified-name Element.getAttribute operation.
    pub fn get_null(&self, name: &str) -> Option<&String> {
        self.namespaced(None, name).map(|entry| &entry.value)
    }
    pub fn contains_null(&self, name: &str) -> bool {
        self.namespaced(None, name).is_some()
    }
    pub fn contains_key(&self, name: &str) -> bool {
        self.named(name).is_some()
    }
    pub fn named(&self, name: &str) -> Option<&ContentAttribute> {
        self.0
            .iter()
            .find(|entry| entry.name == name)
            .map(AsRef::as_ref)
    }
    pub fn namespaced(
        &self,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<&ContentAttribute> {
        let namespace = namespace.filter(|value| !value.is_empty());
        self.0
            .iter()
            .find(|entry| entry.namespace.as_deref() == namespace && entry.local_name == local_name)
            .map(AsRef::as_ref)
    }
    pub(crate) fn shared(
        &self,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<Arc<ContentAttribute>> {
        let namespace = namespace.filter(|value| !value.is_empty());
        self.0
            .iter()
            .find(|entry| entry.namespace.as_deref() == namespace && entry.local_name == local_name)
            .cloned()
    }
    pub(crate) fn insert(&mut self, entry: Arc<ContentAttribute>) -> Option<Arc<ContentAttribute>> {
        if let Some(previous) = self.0.iter_mut().find(|previous| {
            previous.namespace == entry.namespace && previous.local_name == entry.local_name
        }) {
            Some(std::mem::replace(previous, entry))
        } else {
            self.0.push(entry);
            None
        }
    }
    pub(crate) fn remove(&mut self, key: &AttributeKey) -> Option<Arc<ContentAttribute>> {
        let index = self.0.iter().position(|entry| {
            entry.namespace == key.namespace && entry.local_name == key.local_name
        })?;
        Some(self.0.remove(index))
    }
}

impl<'a> IntoIterator for &'a AttributeList {
    type Item = (&'a String, &'a String);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, Arc<ContentAttribute>>,
        fn(&Arc<ContentAttribute>) -> (&String, &String),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|entry| (&entry.name, &entry.value))
    }
}
