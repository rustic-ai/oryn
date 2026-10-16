//! Ordered content attributes. Mutation is restricted to the native DOM crate.
use crate::Node;
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

impl AttributeList {
    pub fn get_null_with_checkpoint<E>(
        &self,
        name: &str,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Option<&String>, E> {
        check()?;
        for entry in &self.0 {
            check()?;
            if entry.namespace.is_none() && checked_attribute_equal(&entry.local_name, name, check)?
            {
                check()?;
                return Ok(Some(&entry.value));
            }
        }
        check()?;
        Ok(None)
    }

    pub fn contains_null_with_checkpoint<E>(
        &self,
        name: &str,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<bool, E> {
        Ok(self.get_null_with_checkpoint(name, check)?.is_some())
    }
}

fn checked_attribute_equal<E>(
    left: &str,
    right: &str,
    check: &mut impl FnMut() -> Result<(), E>,
) -> Result<bool, E> {
    check()?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.bytes().zip(right.bytes()) {
        check()?;
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod attribute_checkpoint_tests {
    use super::*;

    #[test]
    fn cancelled_empty_lookup_and_clone_return_error() {
        let attributes = AttributeList::default();
        assert_eq!(
            attributes.get_null_with_checkpoint("role", &mut || Err("cancel")),
            Err("cancel")
        );
        assert_eq!(
            attributes.clone_with_checkpoint(&mut || Err("cancel")),
            Err("cancel")
        );
    }

    #[test]
    fn null_namespace_order_and_long_name_cancellation() {
        let mut attributes = AttributeList::default();
        let name = "x".repeat(4096);
        attributes.insert(Arc::new(ContentAttribute {
            namespace: Some("foreign".into()),
            local_name: name.clone(),
            name: name.clone(),
            prefix: None,
            value: "foreign".into(),
        }));
        attributes.insert(Arc::new(ContentAttribute {
            namespace: None,
            local_name: name.clone(),
            name: name.clone(),
            prefix: None,
            value: "native".into(),
        }));
        assert_eq!(
            attributes
                .get_null_with_checkpoint(&name, &mut || Ok::<_, &str>(()))
                .unwrap()
                .map(String::as_str),
            Some("native")
        );
        let mut calls = 0;
        assert_eq!(
            attributes.get_null_with_checkpoint(&name, &mut || {
                calls += 1;
                if calls == 64 { Err("cancel") } else { Ok(()) }
            }),
            Err("cancel")
        );
        assert_eq!(calls, 64);
    }
}

/// Internal split keeps existing arbitrary checkpoint-error API contracts.
pub(crate) enum ProducerFailure<C, A> {
    Checkpoint(C),
    Allocation(A),
    AllocationLayout {
        error: A,
        bytes: usize,
        align: usize,
    },
    Capacity,
}
fn shared_attribute_entries<C, A>(
    source: &AttributeList,
    reserve: &mut impl FnMut(usize) -> Result<(), A>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<AttributeList, ProducerFailure<C, A>> {
    check().map_err(ProducerFailure::Checkpoint)?;
    // Attribute counts are already bounded by the existing Vec's allocation,
    // but multiplication overflow still must fail before requesting capacity.
    let bytes = source
        .0
        .len()
        .checked_mul(std::mem::size_of::<Arc<ContentAttribute>>())
        .ok_or(ProducerFailure::Capacity)?;
    reserve(bytes).map_err(ProducerFailure::Allocation)?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(source.0.len()).map_err(|error| {
        ProducerFailure::AllocationLayout {
            error: allocation_error(error),
            bytes,
            align: std::mem::align_of::<Arc<ContentAttribute>>(),
        }
    })?;
    for entry in &source.0 {
        check().map_err(ProducerFailure::Checkpoint)?;
        entries.push(Arc::clone(entry));
    }
    check().map_err(ProducerFailure::Checkpoint)?;
    Ok(AttributeList(entries))
}

fn shared_insert_attribute<C, A>(
    target: &mut AttributeList,
    entry: Arc<ContentAttribute>,
    reserve: &mut impl FnMut(usize) -> Result<(), A>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), ProducerFailure<C, A>> {
    check().map_err(ProducerFailure::Checkpoint)?;
    for previous in &mut target.0 {
        check().map_err(ProducerFailure::Checkpoint)?;
        if previous.namespace.is_none()
            && checked_attribute_equal(&previous.local_name, &entry.local_name, check)
                .map_err(ProducerFailure::Checkpoint)?
        {
            check().map_err(ProducerFailure::Checkpoint)?;
            *previous = entry;
            return Ok(());
        }
    }
    if target.0.len() == target.0.capacity() {
        let needed = target
            .0
            .len()
            .checked_add(1)
            .ok_or(ProducerFailure::Capacity)?;
        let capacity = crate::projection_allocation::growth_capacity(target.0.capacity(), needed)
            .map_err(|_| ProducerFailure::Capacity)?;
        let bytes = (capacity - target.0.capacity())
            .checked_mul(std::mem::size_of::<Arc<ContentAttribute>>())
            .ok_or(ProducerFailure::Capacity)?;
        let total_bytes =
            crate::projection_allocation::checked_capacity_bytes::<Arc<ContentAttribute>>(capacity)
                .map_err(|_| ProducerFailure::Capacity)?;
        reserve(bytes).map_err(ProducerFailure::Allocation)?;
        target
            .0
            .try_reserve_exact(capacity - target.0.len())
            .map_err(|error| ProducerFailure::AllocationLayout {
                error: allocation_error(error),
                bytes: total_bytes,
                align: std::mem::align_of::<Arc<ContentAttribute>>(),
            })?;
    }
    check().map_err(ProducerFailure::Checkpoint)?;
    target.0.push(entry);
    Ok(())
}

fn shared_attribute_string<C, A>(
    source: &str,
    reserve: &mut impl FnMut(usize) -> Result<(), A>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<String, ProducerFailure<C, A>> {
    check().map_err(ProducerFailure::Checkpoint)?;
    reserve(source.len()).map_err(ProducerFailure::Allocation)?;
    let mut output = String::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|error| ProducerFailure::AllocationLayout {
            error: allocation_error(error),
            bytes: source.len(),
            align: 1,
        })?;
    for ch in source.chars() {
        check().map_err(ProducerFailure::Checkpoint)?;
        output.push(ch);
    }
    check().map_err(ProducerFailure::Checkpoint)?;
    Ok(output)
}

pub(crate) fn shared_effective_attributes<'a, C, A>(
    node: &'a Node,
    reserve: &mut impl FnMut(usize) -> Result<(), A>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<std::borrow::Cow<'a, AttributeList>, ProducerFailure<C, A>> {
    check().map_err(ProducerFailure::Checkpoint)?;
    if node.default_accessibility.is_empty() && node.submit_behavior.is_none() {
        check().map_err(ProducerFailure::Checkpoint)?;
        return Ok(std::borrow::Cow::Borrowed(&node.attributes));
    }
    let mut attributes =
        shared_attribute_entries(&node.attributes, reserve, allocation_error, check)?;
    for (key, value) in &node.default_accessibility {
        check().map_err(ProducerFailure::Checkpoint)?;
        if !attributes
            .contains_null_with_checkpoint(key, check)
            .map_err(ProducerFailure::Checkpoint)?
        {
            let entry = shared_effective_entry(key, value, reserve, allocation_error, check)?;
            shared_insert_attribute(&mut attributes, entry, reserve, allocation_error, check)?;
        }
    }
    if let Some(behavior) = &node.submit_behavior
        && !attributes
            .contains_null_with_checkpoint("role", check)
            .map_err(ProducerFailure::Checkpoint)?
    {
        let entry =
            shared_effective_entry("role", &behavior.role, reserve, allocation_error, check)?;
        shared_insert_attribute(&mut attributes, entry, reserve, allocation_error, check)?;
    }
    check().map_err(ProducerFailure::Checkpoint)?;
    Ok(std::borrow::Cow::Owned(attributes))
}
fn shared_effective_entry<C, A>(
    key: &str,
    value: &str,
    reserve: &mut impl FnMut(usize) -> Result<(), A>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<Arc<ContentAttribute>, ProducerFailure<C, A>> {
    let local_name = shared_attribute_string(key, reserve, allocation_error, check)?;
    let name = shared_attribute_string(key, reserve, allocation_error, check)?;
    let value = shared_attribute_string(value, reserve, allocation_error, check)?;
    reserve(std::mem::size_of::<ContentAttribute>() + 2 * std::mem::size_of::<usize>())
        .map_err(ProducerFailure::Allocation)?;
    check().map_err(ProducerFailure::Checkpoint)?;
    Ok(Arc::new(ContentAttribute {
        namespace: None,
        local_name,
        name,
        prefix: None,
        value,
    }))
}

pub(crate) fn ordinary_allocation_failure(bytes: usize, align: usize) -> ! {
    match std::alloc::Layout::from_size_align(bytes.max(1), align) {
        Ok(layout) => std::alloc::handle_alloc_error(layout),
        Err(_) => panic!("ordinary attribute capacity overflow"),
    }
}

impl AttributeList {
    pub fn clone_with_checkpoint<E>(
        &self,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        match shared_attribute_entries(
            self,
            &mut |_| Ok::<_, std::collections::TryReserveError>(()),
            &mut |error| error,
            check,
        ) {
            Ok(value) => Ok(value),
            Err(ProducerFailure::Checkpoint(error)) => Err(error),
            Err(ProducerFailure::Allocation(_)) => ordinary_allocation_failure(1, 1),
            Err(ProducerFailure::AllocationLayout { bytes, align, .. }) => {
                ordinary_allocation_failure(bytes, align)
            }
            Err(ProducerFailure::Capacity) => panic!("ordinary attribute capacity overflow"),
        }
    }
    pub fn clone_with_allocation<A: crate::projection_allocation::ProjectionAllocation>(
        &self,
        allocation: &mut A,
        check: &mut impl FnMut() -> Result<(), crate::DomError>,
    ) -> Result<Self, A::Error> {
        // A single shared RefCell only sequences two closure accesses within
        // the shared algorithm. No callback runs while an allocator borrow lives.
        let allocator = std::cell::RefCell::new(allocation);
        match shared_attribute_entries(
            self,
            &mut |bytes| allocator.borrow_mut().reserve(bytes),
            &mut |error| allocator.borrow().allocation_error(error),
            check,
        ) {
            Ok(value) => Ok(value),
            Err(ProducerFailure::Checkpoint(error)) => Err(A::Error::from(error)),
            Err(ProducerFailure::Allocation(error)) => Err(error),
            Err(ProducerFailure::AllocationLayout { error, .. }) => Err(error),
            Err(ProducerFailure::Capacity) => {
                Err(A::Error::from(crate::DomError::SemanticProjectionCapacity))
            }
        }
    }
}
