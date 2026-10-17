//! Native AX marker ABI verified against WebKit HIServicesSPI.h blob
//! 1f20289bdfed220bcd63025ebda715e4d854e8c8 and installed HIServices.tbd.
//! Markers contain no process pointers or author-callable capability.
use super::super::text_markers::{LENGTH, Marker};
use super::*;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXTextMarkerCreate(allocator: Id, bytes: *const u8, length: isize) -> Id;
    fn AXTextMarkerGetTypeID() -> usize;
    fn AXTextMarkerGetLength(marker: Id) -> isize;
    fn AXTextMarkerGetBytePtr(marker: Id) -> *const u8;
    fn AXTextMarkerRangeCreate(allocator: Id, start: Id, end: Id) -> Id;
    fn AXTextMarkerRangeGetTypeID() -> usize;
    fn AXTextMarkerRangeCopyStartMarker(range: Id) -> Id;
    fn AXTextMarkerRangeCopyEndMarker(range: Id) -> Id;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(value: Id) -> usize;
    fn CFRelease(value: *const c_void);
    fn CFAutorelease(value: Id) -> Id;
}
pub(super) const ATTRIBUTES: &[&str] = &[
    "AXStartTextMarker",
    "AXEndTextMarker",
    "AXSelectedTextMarkerRange",
];
pub(super) const PARAMETERS: &[&str] = &[
    "AXTextMarkerForIndex",
    "AXIndexForTextMarker",
    "AXTextMarkerIsValid",
    "AXUIElementForTextMarker",
    "AXNextTextMarkerForTextMarker",
    "AXPreviousTextMarkerForTextMarker",
    "AXStringForTextMarkerRange",
    "AXLengthForTextMarkerRange",
    "AXBoundsForTextMarkerRange",
    "AXTextMarkerRangeForUnorderedTextMarkers",
    "AXTextMarkerRangeForUIElement",
    "AXNextWordEndTextMarkerForTextMarker",
    "AXPreviousWordStartTextMarkerForTextMarker",
    "AXLeftWordTextMarkerRangeForTextMarker",
    "AXRightWordTextMarkerRangeForTextMarker",
];
unsafe fn payload(value: Id) -> Option<Marker> {
    unsafe {
        if value.is_null()
            || CFGetTypeID(value) != AXTextMarkerGetTypeID()
            || AXTextMarkerGetLength(value) != LENGTH as isize
        {
            return None;
        }
        let bytes = AXTextMarkerGetBytePtr(value);
        if bytes.is_null() {
            return None;
        }
        Marker::decode(std::slice::from_raw_parts(bytes, LENGTH))
    }
}
fn resolve(entry: &Entry, marker: &Marker) -> Option<Marker> {
    marker.resolve(
        entry.marker_namespace,
        entry.node.id,
        entry.marker_scope.as_ref()?,
        entry.node.text.as_ref()?,
        entry.node.boundary_journal.as_deref(),
    )
}
unsafe fn decode(entry: &Entry, value: Id) -> Option<Marker> {
    resolve(entry, &unsafe { payload(value)? })
}
// Resolve within this published native host only. Child documents have their
// own journals and epochs; a root AX query never rebases through a parent journal.
fn owner(entries: &BTreeMap<usize, Entry>, query: &Entry, marker: &Marker) -> Option<usize> {
    let same_host = |candidate: &Entry| {
        candidate.window == query.window && candidate.marker_namespace == query.marker_namespace
    };
    let (id, candidate, resolved) = entries
        .iter()
        .filter(|(_, candidate)| same_host(candidate))
        .filter_map(|(id, candidate)| {
            resolve(candidate, marker).map(|resolved| (*id, candidate, resolved))
        })
        .min_by_key(|(_, candidate, _)| {
            candidate
                .node
                .text
                .as_ref()
                .map_or(usize::MAX, |text| text.value.len())
        })?;
    if let Some(boundary) = resolved.boundary {
        let packed = (u64::from(boundary.start.container.generation) << 32)
            | u64::from(boundary.start.container.slot);
        if let Some((source, _)) = entries.iter().find(|(_, entry)| {
            same_host(entry)
                && entry.node.id.page_id == candidate.node.id.page_id
                && entry.node.id.document_generation == candidate.node.id.document_generation
                && entry.node.id.node == packed
                && entry.marker_scope.as_ref().is_some_and(|scope| {
                    scope.live
                        && scope.realm.0 == resolved.realm
                        && scope.stream_epoch == resolved.epoch
                })
        }) {
            return Some(*source);
        }
    }
    Some(id)
}
// The rendered snapshot supplies exact U+FFFC owner positions. Expand only
// published native children at those positions; never align text by searching
// for equal strings or by sorting allocation addresses.
struct ComposedText {
    units: Vec<u16>,
    origins: Vec<(usize, u32)>,
    points: Vec<(usize, u32)>,
    aliases: BTreeMap<(usize, u32), usize>,
}
fn composed(entries: &BTreeMap<usize, Entry>, query: &Entry) -> Option<ComposedText> {
    let same = |candidate: &Entry| {
        candidate.window == query.window && candidate.marker_namespace == query.marker_namespace
    };
    let mut root = *entries
        .iter()
        .find(|(_, entry)| {
            same(entry)
                && entry.node.id == query.node.id
                && entry
                    .marker_scope
                    .as_ref()
                    .zip(query.marker_scope.as_ref())
                    .is_some_and(|(a, b)| a.realm == b.realm && a.stream_epoch == b.stream_epoch)
        })?
        .0;
    let mut ancestors = std::collections::BTreeSet::new();
    while let Some(parent) = entries.get(&root).and_then(|entry| {
        entries
            .get(&(entry.parent as usize))
            .filter(|entry| same(entry))
            .map(|_| entry.parent as usize)
    }) {
        if !ancestors.insert(root) {
            return None;
        }
        root = parent;
    }
    let mut result = ComposedText {
        units: Vec::new(),
        origins: Vec::new(),
        points: Vec::new(),
        aliases: BTreeMap::new(),
    };
    let mut seen = std::collections::BTreeSet::new();
    fn append(
        entries: &BTreeMap<usize, Entry>,
        query: &Entry,
        id: usize,
        result: &mut ComposedText,
        seen: &mut std::collections::BTreeSet<usize>,
        depth: usize,
    ) -> Option<bool> {
        if depth > 512 {
            return None;
        }
        if !seen.insert(id) || seen.len() > 262144 {
            return None;
        }
        let entry = entries.get(&id)?;
        if entry.window != query.window || entry.marker_namespace != query.marker_namespace {
            return None;
        }
        let Some(text) = entry.node.text.as_ref().filter(|text| !text.protected) else {
            let mut found = false;
            for child in &entry.children {
                found |= append(entries, query, *child as usize, result, seen, depth + 1)?;
            }
            return Some(found);
        };
        if !entry.marker_scope.as_ref().is_some_and(|scope| scope.live) {
            return None;
        }
        let mut objects = BTreeMap::new();
        for object in &text.objects {
            let offset = object.offset as usize;
            if text.value.get(offset) != Some(&0xfffc)
                || objects.insert(offset, object.node).is_some()
            {
                return None;
            }
        }
        if result.points.is_empty() {
            result.points.push((id, 0));
        }
        for offset in 0..text.value.len() {
            result
                .aliases
                .insert((id, offset as u32), result.units.len());
            let mut expanded = false;
            if let Some(node) = objects.get(&offset) {
                let packed = (u64::from(node.generation) << 32) | u64::from(node.slot);
                if let Some((child, _)) = entries.iter().find(|(_, candidate)| {
                    candidate.window == query.window
                        && candidate.marker_namespace == query.marker_namespace
                        && candidate.node.id.node == packed
                        && candidate.node.id.page_id == entry.node.id.page_id
                        && candidate.node.id.document_generation
                            == entry.node.id.document_generation
                        && candidate
                            .marker_scope
                            .as_ref()
                            .zip(entry.marker_scope.as_ref())
                            .is_some_and(|(a, b)| {
                                a.realm == b.realm && a.stream_epoch == b.stream_epoch
                            })
                }) && *child != id
                {
                    expanded = append(entries, query, *child, result, seen, depth + 1)?;
                }
            }
            if !expanded {
                if result.units.len() >= 2 * 1024 * 1024 {
                    return None;
                }
                if let Some(last) = result.points.last_mut() {
                    *last = (id, offset as u32);
                }
                result.origins.push((id, offset as u32));
                result.units.push(text.value[offset]);
                result.points.push((id, offset as u32 + 1));
            }
        }
        result
            .aliases
            .insert((id, text.value.len() as u32), result.units.len());
        Some(true)
    }
    append(entries, query, root, &mut result, &mut seen, 0)?;
    Some(result)
}
impl ComposedText {
    fn index(&self, entries: &BTreeMap<usize, Entry>, marker: &Marker) -> Option<usize> {
        entries.iter().find_map(|(id, entry)| {
            resolve(entry, marker)
                .and_then(|resolved| self.aliases.get(&(*id, resolved.offset)).copied())
        })
    }
    unsafe fn marker(&self, entries: &BTreeMap<usize, Entry>, offset: usize) -> Id {
        unsafe {
            self.points
                .get(offset)
                .and_then(|(id, offset)| entries.get(id).map(|entry| marker(entry, *offset)))
                .unwrap_or(std::ptr::null_mut())
        }
    }
}
unsafe fn range_payload(value: Id) -> Option<(Marker, Marker)> {
    unsafe {
        if value.is_null() || CFGetTypeID(value) != AXTextMarkerRangeGetTypeID() {
            return None;
        }
        let first = AXTextMarkerRangeCopyStartMarker(value);
        let last = AXTextMarkerRangeCopyEndMarker(value);
        let result = payload(first).zip(payload(last));
        if !first.is_null() {
            CFRelease(first);
        }
        if !last.is_null() {
            CFRelease(last);
        }
        result
    }
}
unsafe fn composed_range(
    text: &ComposedText,
    entries: &BTreeMap<usize, Entry>,
    start: usize,
    end: usize,
) -> Id {
    unsafe {
        let first = text.marker(entries, start);
        let last = text.marker(entries, end);
        if first.is_null() || last.is_null() {
            return std::ptr::null_mut();
        }
        let result = AXTextMarkerRangeCreate(std::ptr::null_mut(), first, last);
        if result.is_null() {
            result
        } else {
            CFAutorelease(result)
        }
    }
}
unsafe fn create(entry: &Entry, offset: u32) -> Id {
    unsafe {
        let Some(marker) = entry.marker_scope.as_ref().and_then(|scope| {
            Marker::new(
                entry.marker_namespace,
                entry.node.id,
                scope,
                entry.node.text.as_ref()?,
                offset,
            )
        }) else {
            return std::ptr::null_mut();
        };
        let bytes = marker.encode();
        AXTextMarkerCreate(std::ptr::null_mut(), bytes.as_ptr(), bytes.len() as isize)
    }
}
unsafe fn marker(entry: &Entry, offset: u32) -> Id {
    unsafe {
        let marker = create(entry, offset);
        if marker.is_null() {
            marker
        } else {
            CFAutorelease(marker)
        }
    }
}
unsafe fn range(entry: &Entry, start: u32, end: u32) -> Id {
    unsafe {
        let first = create(entry, start);
        if first.is_null() {
            return first;
        }
        let last = create(entry, end);
        if last.is_null() {
            CFRelease(first);
            return last;
        }
        let range = AXTextMarkerRangeCreate(std::ptr::null_mut(), first, last);
        CFRelease(first);
        CFRelease(last);
        if range.is_null() {
            range
        } else {
            CFAutorelease(range)
        }
    }
}
unsafe fn endpoints(entry: &Entry, value: Id) -> Option<(u32, u32)> {
    unsafe {
        if value.is_null() || CFGetTypeID(value) != AXTextMarkerRangeGetTypeID() {
            return None;
        }
        let first = AXTextMarkerRangeCopyStartMarker(value);
        let last = AXTextMarkerRangeCopyEndMarker(value);
        let result = decode(entry, first)
            .zip(decode(entry, last))
            .map(|(first, last)| (first.offset, last.offset));
        if !first.is_null() {
            CFRelease(first);
        }
        if !last.is_null() {
            CFRelease(last);
        }
        result
    }
}
pub(super) unsafe fn attribute(entry: &Entry, name: &str) -> Option<Id> {
    if !ATTRIBUTES.contains(&name) {
        return None;
    }
    Some(unsafe {
        if let Some(text) = entry.node.text.as_ref().filter(|text| !text.protected) {
            match name {
                "AXStartTextMarker" => marker(entry, 0),
                "AXEndTextMarker" => u32::try_from(text.value.len())
                    .ok()
                    .map_or(std::ptr::null_mut(), |end| marker(entry, end)),
                "AXSelectedTextMarkerRange" => range(entry, text.start, text.end),
                _ => std::ptr::null_mut(),
            }
        } else {
            std::ptr::null_mut()
        }
    })
}
pub(super) unsafe fn selection(entry: &Entry, value: Id) -> Option<Action> {
    unsafe {
        let (start, end) = endpoints(entry, value)?;
        let expected = entry.node.text.as_ref()?.clone();
        Some(Action::SelectText {
            start: start.min(end),
            end: start.max(end),
            expected,
        })
    }
}
unsafe fn composed_parameter(
    entries: &BTreeMap<usize, Entry>,
    entry: &Entry,
    name: &str,
    value: Id,
) -> Option<Id> {
    unsafe {
        let relevant = matches!(
            name,
            "AXStringForTextMarkerRange"
                | "AXLengthForTextMarkerRange"
                | "AXBoundsForTextMarkerRange"
                | "AXTextMarkerRangeForUnorderedTextMarkers"
                | "AXNextTextMarkerForTextMarker"
                | "AXPreviousTextMarkerForTextMarker"
                | "AXNextWordEndTextMarkerForTextMarker"
                | "AXPreviousWordStartTextMarkerForTextMarker"
                | "AXLeftWordTextMarkerRangeForTextMarker"
                | "AXRightWordTextMarkerRangeForTextMarker"
        );
        if !relevant {
            return None;
        }
        let text = composed(entries, entry)?;
        if name == "AXTextMarkerRangeForUnorderedTextMarkers" {
            let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            let count: unsafe extern "C" fn(Id, Sel) -> usize =
                std::mem::transmute(objc_msgSend as *const ());
            let at: unsafe extern "C" fn(Id, Sel, usize) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            if value.is_null()
                || check(value, sel("isKindOfClass:"), class("NSArray")) == 0
                || count(value, sel("count")) != 2
            {
                return Some(std::ptr::null_mut());
            }
            let a = payload(at(value, sel("objectAtIndex:"), 0))
                .and_then(|marker| text.index(entries, &marker))?;
            let b = payload(at(value, sel("objectAtIndex:"), 1))
                .and_then(|marker| text.index(entries, &marker))?;
            return Some(composed_range(&text, entries, a.min(b), a.max(b)));
        }
        if matches!(
            name,
            "AXStringForTextMarkerRange"
                | "AXLengthForTextMarkerRange"
                | "AXBoundsForTextMarkerRange"
        ) {
            let (a, b) = range_payload(value)?;
            let a = text.index(entries, &a)?;
            let b = text.index(entries, &b)?;
            let (start, end) = (a.min(b), a.max(b));
            if name == "AXStringForTextMarkerRange" {
                return Some(utf16(text.units.get(start..end)?));
            }
            if name == "AXLengthForTextMarkerRange" {
                return Some(integer(end - start));
            }
            let mut bounds: Option<(f64, f64, f64, f64)> = None;
            let mut add = |owner: &Entry, r: oryn_common::v4::CssRect| {
                let left = owner.root_origin.x + r.x;
                let bottom = owner.root_origin.y + owner.viewport_height - r.bottom();
                let right = left + r.width;
                let top = bottom + r.height;
                if [left, bottom, right, top].iter().all(|v| v.is_finite()) {
                    bounds = Some(match bounds {
                        Some((l, b, r, t)) => {
                            (l.min(left), b.min(bottom), r.max(right), t.max(top))
                        }
                        None => (left, bottom, right, top),
                    });
                }
            };
            if start == end {
                let (id, offset) = *text.points.get(start)?;
                let owner = entries.get(&id)?;
                if let Some(r) = owner
                    .node
                    .text
                    .as_ref()?
                    .carets
                    .iter()
                    .find(|caret| caret.offset == offset)
                    .and_then(|caret| caret.rect)
                {
                    add(owner, r);
                }
            } else {
                let mut ranges: BTreeMap<usize, (u32, u32)> = BTreeMap::new();
                for (id, offset) in text.origins.get(start..end)? {
                    ranges
                        .entry(*id)
                        .and_modify(|range| {
                            range.0 = range.0.min(*offset);
                            range.1 = range.1.max(offset + 1);
                        })
                        .or_insert((*offset, offset + 1));
                }
                for (id, (start, end)) in ranges {
                    let owner = entries.get(&id)?;
                    for item in &owner.node.text.as_ref()?.boxes {
                        if item.start < end && item.end > start {
                            add(owner, item.rect);
                        }
                    }
                }
            }
            return Some(
                bounds.map_or(std::ptr::null_mut(), |(left, bottom, right, top)| {
                    rect(Rect {
                        origin: Point { x: left, y: bottom },
                        size: Size {
                            width: right - left,
                            height: top - bottom,
                        },
                    })
                }),
            );
        }
        let current = text.index(entries, &payload(value)?)?;
        let current = u32::try_from(current).ok()?;
        if matches!(
            name,
            "AXLeftWordTextMarkerRangeForTextMarker" | "AXRightWordTextMarkerRangeForTextMarker"
        ) {
            let (start, end) = oryn_js::accessibility::touching_word_range(
                &text.units,
                current,
                name == "AXRightWordTextMarkerRangeForTextMarker",
            )?;
            return Some(composed_range(&text, entries, start as usize, end as usize));
        }
        let forward = matches!(
            name,
            "AXNextTextMarkerForTextMarker" | "AXNextWordEndTextMarkerForTextMarker"
        );
        let next = if matches!(
            name,
            "AXNextTextMarkerForTextMarker" | "AXPreviousTextMarkerForTextMarker"
        ) {
            oryn_js::accessibility::adjacent_grapheme(&text.units, current, forward)
        } else {
            oryn_js::accessibility::adjacent_word_boundary(&text.units, current, forward)
        };
        Some(next.map_or(std::ptr::null_mut(), |next| {
            text.marker(entries, next as usize)
        }))
    }
}
pub(super) unsafe fn parameter(object: Id, entry: &Entry, name: &str, value: Id) -> Option<Id> {
    if !PARAMETERS.contains(&name) {
        return None;
    }
    Some(unsafe {
        if matches!(name, "AXTextMarkerIsValid" | "AXUIElementForTextMarker") {
            let found = payload(value).and_then(|marker| {
                ENTRIES.with(|entries| {
                    entries
                        .try_borrow()
                        .ok()
                        .and_then(|entries| owner(&entries, entry, &marker))
                })
            });
            return Some(if name == "AXTextMarkerIsValid" {
                number(found.is_some() || decode(entry, value).is_some())
            } else {
                found.map_or_else(
                    || decode(entry, value).map_or(std::ptr::null_mut(), |_| object),
                    |id| id as Id,
                )
            });
        }
        if name == "AXTextMarkerRangeForUIElement" && value != object {
            return Some(ENTRIES.with(|entries| {
                entries
                    .try_borrow()
                    .ok()
                    .and_then(|entries| {
                        let target = entries.get(&(value as usize))?;
                        if target.window != entry.window
                            || target.marker_namespace != entry.marker_namespace
                        {
                            return None;
                        }
                        let text = target.node.text.as_ref().filter(|text| !text.protected)?;
                        Some(range(target, 0, u32::try_from(text.value.len()).ok()?))
                    })
                    .unwrap_or(std::ptr::null_mut())
            }));
        }
        if let Some(result) = ENTRIES.with(|entries| {
            entries
                .try_borrow()
                .ok()
                .and_then(|entries| composed_parameter(&entries, entry, name, value))
        }) {
            return Some(result);
        }
        let Some(text) = entry.node.text.as_ref().filter(|text| !text.protected) else {
            return Some(std::ptr::null_mut());
        };
        match name {
            "AXTextMarkerForIndex" => get_integer(value)
                .and_then(|index| u32::try_from(index).ok())
                .map_or(std::ptr::null_mut(), |index| marker(entry, index)),
            "AXTextMarkerIsValid" => number(decode(entry, value).is_some()),
            "AXIndexForTextMarker" => decode(entry, value).map_or(std::ptr::null_mut(), |marker| {
                integer(marker.offset as usize)
            }),
            "AXUIElementForTextMarker" => {
                decode(entry, value).map_or(std::ptr::null_mut(), |_| object)
            }
            "AXNextTextMarkerForTextMarker" | "AXPreviousTextMarkerForTextMarker" => {
                let Some(current) = decode(entry, value) else {
                    return Some(std::ptr::null_mut());
                };
                oryn_js::accessibility::adjacent_grapheme(
                    &text.value,
                    current.offset,
                    name == "AXNextTextMarkerForTextMarker",
                )
                .map_or(std::ptr::null_mut(), |offset| marker(entry, offset))
            }
            "AXNextWordEndTextMarkerForTextMarker"
            | "AXPreviousWordStartTextMarkerForTextMarker" => {
                let Some(current) = decode(entry, value) else {
                    return Some(std::ptr::null_mut());
                };
                oryn_js::accessibility::adjacent_word_boundary(
                    &text.value,
                    current.offset,
                    name == "AXNextWordEndTextMarkerForTextMarker",
                )
                .map_or(std::ptr::null_mut(), |offset| marker(entry, offset))
            }
            "AXLeftWordTextMarkerRangeForTextMarker"
            | "AXRightWordTextMarkerRangeForTextMarker" => {
                let Some(current) = decode(entry, value) else {
                    return Some(std::ptr::null_mut());
                };
                oryn_js::accessibility::touching_word_range(
                    &text.value,
                    current.offset,
                    name == "AXRightWordTextMarkerRangeForTextMarker",
                )
                .map_or(std::ptr::null_mut(), |(start, end)| {
                    range(entry, start, end)
                })
            }
            "AXTextMarkerRangeForUIElement" => {
                if value == object {
                    range(entry, 0, text.value.len() as u32)
                } else {
                    std::ptr::null_mut()
                }
            }
            "AXTextMarkerRangeForUnorderedTextMarkers" => {
                let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
                    std::mem::transmute(objc_msgSend as *const ());
                let count: unsafe extern "C" fn(Id, Sel) -> usize =
                    std::mem::transmute(objc_msgSend as *const ());
                let at: unsafe extern "C" fn(Id, Sel, usize) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                if value.is_null()
                    || check(value, sel("isKindOfClass:"), class("NSArray")) == 0
                    || count(value, sel("count")) != 2
                {
                    return Some(std::ptr::null_mut());
                }
                decode(entry, at(value, sel("objectAtIndex:"), 0))
                    .zip(decode(entry, at(value, sel("objectAtIndex:"), 1)))
                    .map_or(std::ptr::null_mut(), |(a, b)| {
                        range(entry, a.offset.min(b.offset), a.offset.max(b.offset))
                    })
            }
            "AXStringForTextMarkerRange"
            | "AXLengthForTextMarkerRange"
            | "AXBoundsForTextMarkerRange" => {
                let Some((start, end)) = endpoints(entry, value) else {
                    return Some(std::ptr::null_mut());
                };
                let (start, end) = (start.min(end) as usize, start.max(end) as usize);
                match name {
                    "AXStringForTextMarkerRange" => text
                        .value
                        .get(start..end)
                        .map_or(std::ptr::null_mut(), |units| utf16(units)),
                    "AXLengthForTextMarkerRange" => integer(end - start),
                    _ => super::parameter(
                        object,
                        std::ptr::null_mut(),
                        string("AXBoundsForRange"),
                        super::range(Range {
                            location: start,
                            length: end - start,
                        }),
                    ),
                }
            }
            _ => std::ptr::null_mut(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        let snapshot = super::super::super::tests::fixture();
        let mut node = snapshot.nodes.into_values().next().unwrap();
        node.text = Some(oryn_js::accessibility::TextSnapshot {
            objects: Vec::new(),
            control: None,
            boundary_revision: 0,
            boundaries: Vec::new(),
            protected_caret: None,
            value: vec![97, 0xd800, 0xd83d, 0xde00],
            start: 1,
            end: 2,
            selection_revision: 1,
            protected: false,
            boxes: vec![],
            carets: vec![],
        });
        let scope = crate::host::webdriver_script::AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(1),
            parent_realm: None,
            frame: None,
            frame_generation: None,
            frame_token: None,
            document_root: oryn_dom::NodeId {
                slot: 1,
                generation: 1,
            },
            document_generation: node.id.document_generation,
            stream_epoch: 1,
            document_authority_epoch: 0,
            url: "https://example.test/".into(),
            live: true,
        };
        let (sender, _) = std::sync::mpsc::sync_channel(1);
        Entry {
            marker_namespace: [9; 16],
            marker_scope: Some(scope),
            table: None,
            table_objects: Rc::new(BTreeMap::new()),
            revision: node.revision,
            node,
            parent: std::ptr::null_mut(),
            children: vec![],
            relations: BTreeMap::new(),
            window: std::ptr::null_mut(),
            screen: None,
            root_origin: Point { x: 0., y: 0. },
            viewport_height: 100.,
            sender: crate::owned_action_request::NativeActionSender::Plain(sender),
        }
    }
    #[test]
    fn composed_ranges_use_exact_frame_object_positions_and_child_epochs() {
        let mut root = entry();
        let mut frame = entry();
        let mut child = entry();
        let frame_node = oryn_dom::NodeId {
            slot: 20,
            generation: 1,
        };
        frame.node.id.node = (1u64 << 32) | 20;
        frame.node.text = None;
        frame.parent = 1usize as Id;
        frame.children = vec![3usize as Id];
        child.node.id.node = (1u64 << 32) | 30;
        child.parent = 2usize as Id;
        child.marker_scope.as_mut().unwrap().realm = oryn_js::realm::RealmId(2);
        child.node.text.as_mut().unwrap().value = vec![85, 86];
        root.children = vec![2usize as Id];
        let text = root.node.text.as_mut().unwrap();
        text.value = vec![65, 0xfffc, 66];
        text.objects = vec![oryn_js::accessibility::TextObject {
            offset: 1,
            node: frame_node,
        }];
        let child_marker = Marker::new(
            child.marker_namespace,
            child.node.id,
            child.marker_scope.as_ref().unwrap(),
            child.node.text.as_ref().unwrap(),
            1,
        )
        .unwrap();
        let root_marker = Marker::new(
            root.marker_namespace,
            root.node.id,
            root.marker_scope.as_ref().unwrap(),
            root.node.text.as_ref().unwrap(),
            2,
        )
        .unwrap();
        let mut entries = BTreeMap::from([(1, root), (2, frame), (3, child)]);
        let text = composed(&entries, &entries[&1]).unwrap();
        assert_eq!(text.units, vec![65, 85, 86, 66]);
        assert_eq!(text.index(&entries, &child_marker), Some(2));
        assert_eq!(text.index(&entries, &root_marker), Some(3));
        entries
            .get_mut(&3)
            .unwrap()
            .marker_scope
            .as_mut()
            .unwrap()
            .stream_epoch += 1;
        assert!(
            composed(&entries, &entries[&1])
                .unwrap()
                .index(&entries, &child_marker)
                .is_none()
        );
        entries
            .get_mut(&3)
            .unwrap()
            .node
            .text
            .as_mut()
            .unwrap()
            .protected = true;
        assert_eq!(
            composed(&entries, &entries[&1]).unwrap().units,
            vec![65, 0xfffc, 66]
        );
    }
    #[test]
    fn composed_ranges_reject_forged_object_offsets_and_publication_cycles() {
        let mut root = entry();
        root.node.text.as_mut().unwrap().objects = vec![oryn_js::accessibility::TextObject {
            offset: 0,
            node: oryn_dom::NodeId {
                slot: 20,
                generation: 1,
            },
        }];
        let entries = BTreeMap::from([(1, root)]);
        assert!(composed(&entries, &entries[&1]).is_none());
        let mut root = entry();
        root.node.text = None;
        root.children = vec![1usize as Id];
        let entries = BTreeMap::from([(1, root)]);
        assert!(composed(&entries, &entries[&1]).is_none());
    }
    #[test]
    fn real_cf_markers_validate_shape_lifetime_and_protected_text() {
        unsafe {
            let mut entry = entry();
            let marker = create(&entry, 2);
            assert!(!marker.is_null());
            assert_eq!(decode(&entry, marker).unwrap().offset, 2);
            entry.marker_scope.as_mut().unwrap().stream_epoch += 1;
            assert!(decode(&entry, marker).is_none());
            entry.marker_scope.as_mut().unwrap().stream_epoch -= 1;
            entry.node.text.as_mut().unwrap().protected = true;
            assert!(decode(&entry, marker).is_none());
            assert!(create(&entry, 0).is_null());
            CFRelease(marker);
            let short = AXTextMarkerCreate(std::ptr::null_mut(), [0u8].as_ptr(), 1);
            assert!(decode(&entry, short).is_none());
            if !short.is_null() {
                CFRelease(short);
            }
        }
    }
    #[test]
    fn native_marker_ranges_preserve_exact_surrogate_offsets() {
        unsafe {
            let entry = entry();
            let a = create(&entry, 1);
            let b = create(&entry, 3);
            let value = AXTextMarkerRangeCreate(std::ptr::null_mut(), a, b);
            assert_eq!(endpoints(&entry, value), Some((1, 3)));
            match selection(&entry, value).unwrap() {
                Action::SelectText {
                    start,
                    end,
                    expected,
                } => {
                    assert_eq!((start, end), (1, 3));
                    assert_eq!(expected.value, vec![97, 0xd800, 0xd83d, 0xde00]);
                }
                _ => panic!("wrong native marker action"),
            };
            CFRelease(value);
            CFRelease(a);
            CFRelease(b);
        }
    }
    #[test]
    fn native_marker_navigation_skips_whole_graphemes() {
        unsafe {
            let mut entry = entry();
            entry.node.text.as_mut().unwrap().value = "a\u{301}😀".encode_utf16().collect();
            let pool = object(class("NSAutoreleasePool"), "new");
            let first = create(&entry, 0);
            let next = parameter(
                std::ptr::null_mut(),
                &entry,
                "AXNextTextMarkerForTextMarker",
                first,
            )
            .unwrap();
            assert_eq!(decode(&entry, next).unwrap().offset, 2);
            let last = parameter(
                std::ptr::null_mut(),
                &entry,
                "AXNextTextMarkerForTextMarker",
                next,
            )
            .unwrap();
            assert_eq!(decode(&entry, last).unwrap().offset, 4);
            let previous = parameter(
                std::ptr::null_mut(),
                &entry,
                "AXPreviousTextMarkerForTextMarker",
                last,
            )
            .unwrap();
            assert_eq!(decode(&entry, previous).unwrap().offset, 2);
            assert!(
                parameter(
                    std::ptr::null_mut(),
                    &entry,
                    "AXNextTextMarkerForTextMarker",
                    last
                )
                .unwrap()
                .is_null()
            );
            CFRelease(first);
            void(pool, "drain");
        }
    }

    #[test]
    fn marker_owner_search_uses_child_lifetime_and_host_namespace() {
        let query = entry();
        let mut child = entry();
        child.node.id.node = 42;
        child.marker_scope.as_mut().unwrap().realm = oryn_js::realm::RealmId(2);
        let marker = Marker::new(
            child.marker_namespace,
            child.node.id,
            child.marker_scope.as_ref().unwrap(),
            child.node.text.as_ref().unwrap(),
            1,
        )
        .unwrap();
        let mut entries = BTreeMap::new();
        entries.insert(42, child);
        assert_eq!(owner(&entries, &query, &marker), Some(42));
        entries
            .get_mut(&42)
            .unwrap()
            .marker_scope
            .as_mut()
            .unwrap()
            .stream_epoch += 1;
        assert_eq!(owner(&entries, &query, &marker), None);
        entries
            .get_mut(&42)
            .unwrap()
            .marker_scope
            .as_mut()
            .unwrap()
            .stream_epoch -= 1;
        entries.get_mut(&42).unwrap().marker_namespace = [3; 16];
        assert_eq!(owner(&entries, &query, &marker), None);
    }
    #[test]
    fn marker_owner_prefers_actual_published_source_over_document_container() {
        let query = entry();
        let mut document = entry();
        let source = oryn_dom::NodeId {
            slot: 55,
            generation: 2,
        };
        let point = oryn_dom::BoundaryPoint {
            container: source,
            offset: 0,
        };
        let text = document.node.text.as_mut().unwrap();
        text.value.clear();
        text.boundaries = vec![oryn_js::accessibility::SourceBoundary {
            start: point,
            end: point,
        }];
        document.node.boundary_journal =
            Some(std::sync::Arc::new(oryn_dom::boundary_journal::Snapshot {
                oldest: 1,
                current: 0,
                complete: true,
                records: vec![],
            }));
        let marker = Marker::new(
            document.marker_namespace,
            document.node.id,
            document.marker_scope.as_ref().unwrap(),
            document.node.text.as_ref().unwrap(),
            0,
        )
        .unwrap();
        let mut leaf = entry();
        leaf.node.id.node = (u64::from(source.generation) << 32) | u64::from(source.slot);
        leaf.node.text = None;
        let mut entries = BTreeMap::new();
        entries.insert(1, document);
        entries.insert(2, leaf);
        assert_eq!(owner(&entries, &query, &marker), Some(2));
        entries.remove(&2);
        assert_eq!(owner(&entries, &query, &marker), Some(1));
    }
}
