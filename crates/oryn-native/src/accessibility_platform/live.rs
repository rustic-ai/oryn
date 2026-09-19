//! Snapshot-only live announcements; native IDs distinguish insertion from text
//! replacement. Busy regions retain the last delivered baseline until released.
use super::*;
const MAX_TEXT: usize = 64 * 1024;
#[derive(Clone, Copy, Debug)]
pub struct Relevant {
    additions: bool,
    text: bool,
    removals: bool,
}
impl Default for Relevant {
    fn default() -> Self {
        Self {
            additions: true,
            text: true,
            removals: false,
        }
    }
}
impl Relevant {
    fn parse(value: &str) -> Self {
        let mut result = Self {
            additions: false,
            text: false,
            removals: false,
        };
        for token in value.split_ascii_whitespace() {
            match token {
                "all" => {
                    return Self {
                        additions: true,
                        text: true,
                        removals: true,
                    };
                }
                "additions" => result.additions = true,
                "text" => result.text = true,
                "removals" => result.removals = true,
                _ => {}
            }
        }
        result
    }
}
#[derive(Clone, Debug)]
pub struct Chunk {
    pub id: SemanticRef,
    pub text: String,
    pub relevant: Relevant,
    pub group: Option<SemanticRef>,
    pub busy: bool,
}
#[derive(Clone, Debug)]
pub struct LiveRegion {
    pub text: String,
    pub chunks: Vec<Chunk>,
    pub assertive: bool,
    pub busy: bool,
    pub atomic: bool,
    pub initial_alert: bool,
    pub groups: BTreeMap<SemanticRef, String>,
    pub blocked_groups: BTreeSet<SemanticRef>,
    pub parents: BTreeMap<SemanticRef, Option<SemanticRef>>,
    pub busy_nodes: BTreeSet<SemanticRef>,
}
#[derive(Clone, Debug)]
pub struct Announcement {
    pub text: String,
    pub assertive: bool,
}
fn inherited<'a>(
    nodes: &'a BTreeMap<SemanticRef, Node>,
    id: SemanticRef,
    key: &str,
) -> Option<&'a serde_json::Value> {
    let mut cursor = Some(id);
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return None;
        }
        let node = nodes.get(&id)?;
        if let Some(value) = node.properties.get(key) {
            return Some(value);
        }
        cursor = node.parent;
    }
    None
}
fn busy(nodes: &BTreeMap<SemanticRef, Node>, id: SemanticRef) -> bool {
    let mut cursor = Some(id);
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return true;
        }
        let Some(node) = nodes.get(&id) else {
            break;
        };
        if node
            .properties
            .get("busy")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            return true;
        }
        cursor = node.parent;
    }
    false
}
fn boundary(node: &Node) -> bool {
    node.properties
        .get("live")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| matches!(value, "off" | "polite" | "assertive"))
        || matches!(
            node.role.as_str(),
            "alert" | "status" | "log" | "marquee" | "timer"
        )
}
fn relevant(nodes: &BTreeMap<SemanticRef, Node>, id: SemanticRef) -> Relevant {
    inherited(nodes, id, "relevant")
        .and_then(serde_json::Value::as_str)
        .map(Relevant::parse)
        .unwrap_or_default()
}
struct Contributions {
    chunks: Vec<Chunk>,
    groups: BTreeMap<SemanticRef, String>,
    blocked_groups: BTreeSet<SemanticRef>,
    parents: BTreeMap<SemanticRef, Option<SemanticRef>>,
    busy_nodes: BTreeSet<SemanticRef>,
}
fn text(
    nodes: &BTreeMap<SemanticRef, Node>,
    root: SemanticRef,
    atomic: bool,
) -> Result<Contributions, String> {
    enum Visit {
        Enter(SemanticRef, Option<SemanticRef>),
        Exit(SemanticRef, usize),
    }
    let mut stack = vec![Visit::Enter(root, atomic.then_some(root))];
    let mut seen = BTreeSet::new();
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut spans = BTreeMap::new();
    let mut parents = BTreeMap::new();
    let mut busy_nodes = BTreeSet::new();
    let mut bytes = 0usize;
    while let Some(visit) = stack.pop() {
        let (id, group) = match visit {
            Visit::Exit(id, start) => {
                spans.insert(id, (start, chunks.len()));
                continue;
            }
            Visit::Enter(id, group) => (id, group),
        };
        if !seen.insert(id) {
            return Err("live region descendant cycle".into());
        }
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        if id != root && boundary(node) {
            continue;
        }
        parents.insert(id, node.parent);
        if node
            .properties
            .get("busy")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            busy_nodes.insert(id);
        }
        let group = match node
            .properties
            .get("atomic")
            .and_then(serde_json::Value::as_bool)
        {
            Some(true) => Some(id),
            Some(false) => None,
            None => group,
        };
        stack.push(Visit::Exit(id, chunks.len()));
        let children = node
            .children
            .iter()
            .filter(|id| nodes.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        let value = if node.text.as_ref().is_some_and(|text| text.protected) {
            None
        } else if matches!(
            node.role.as_str(),
            "textbox" | "searchbox" | "combobox" | "slider" | "spinbutton"
        ) {
            node.value.as_ref().or(Some(&node.name))
        } else if children.is_empty() {
            Some(&node.name)
        } else {
            None
        };
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            bytes = bytes
                .checked_add(value.len())
                .ok_or("live announcement size overflow")?;
            if bytes > MAX_TEXT {
                return Err("live announcement text budget exceeded".into());
            }
            chunks.push(Chunk {
                id,
                text: value.clone(),
                relevant: relevant(nodes, id),
                group,
                busy: busy(nodes, id),
            });
        } else {
            stack.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| Visit::Enter(child, group)),
            );
        }
    }
    let mut groups = BTreeMap::new();
    let mut blocked_groups = BTreeSet::new();
    let mut group_bytes = 0usize;
    for group in chunks
        .iter()
        .filter_map(|chunk| chunk.group)
        .collect::<BTreeSet<_>>()
    {
        let (start, end) = spans
            .get(&group)
            .copied()
            .ok_or("live atomic group has no native span")?;
        let members = &chunks[start..end];
        let value = members
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        group_bytes = group_bytes
            .checked_add(value.len())
            .ok_or("live atomic text size overflow")?;
        if group_bytes > 4 * 1024 * 1024 {
            return Err("live atomic text budget exceeded".into());
        }
        if members.iter().any(|chunk| chunk.busy) {
            blocked_groups.insert(group);
        }
        groups.insert(group, value);
    }
    Ok(Contributions {
        chunks,
        groups,
        blocked_groups,
        parents,
        busy_nodes,
    })
}
pub(super) fn project(
    nodes: &BTreeMap<SemanticRef, Node>,
    roots: impl Iterator<Item = SemanticRef>,
) -> Result<BTreeMap<SemanticRef, LiveRegion>, String> {
    let mut result = BTreeMap::new();
    let mut total_bytes = 0usize;
    for id in roots {
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        let explicit = node
            .properties
            .get("live")
            .and_then(serde_json::Value::as_str);
        let politeness = explicit.unwrap_or(match node.role.as_str() {
            "alert" => "assertive",
            "status" | "log" => "polite",
            _ => "off",
        });
        if !matches!(politeness, "polite" | "assertive") {
            continue;
        }
        let atomic = inherited(nodes, id, "atomic")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(matches!(node.role.as_str(), "alert" | "status"));
        let content = text(nodes, id, atomic)?;
        let text = content
            .chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if text.len() > MAX_TEXT {
            return Err("live announcement text budget exceeded".into());
        }
        total_bytes = total_bytes
            .checked_add(text.len())
            .and_then(|bytes| {
                content
                    .groups
                    .values()
                    .try_fold(bytes, |sum, text| sum.checked_add(text.len()))
            })
            .ok_or("live snapshot text size overflow")?;
        if total_bytes > 4 * 1024 * 1024 {
            return Err("live snapshot text budget exceeded".into());
        }
        result.insert(
            id,
            LiveRegion {
                text,
                chunks: content.chunks,
                groups: content.groups,
                blocked_groups: content.blocked_groups,
                parents: content.parents,
                busy_nodes: content.busy_nodes,
                assertive: politeness == "assertive",
                busy: busy(nodes, id),
                atomic: false,
                initial_alert: node.role == "alert",
            },
        );
    }
    Ok(result)
}
#[derive(Default)]
pub struct Tracker {
    previous: BTreeMap<SemanticRef, LiveRegion>,
}
fn deferred(chunk: &Chunk, region: &LiveRegion, old: Option<&LiveRegion>) -> bool {
    if chunk.busy
        && region
            .chunks
            .iter()
            .any(|current| current.id == chunk.id && current.busy)
    {
        return true;
    }
    if chunk
        .group
        .is_some_and(|group| region.blocked_groups.contains(&group))
    {
        return true;
    }
    let mut cursor = Some(chunk.id);
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return true;
        }
        if region.busy_nodes.contains(&id) {
            return true;
        }
        cursor = region
            .parents
            .get(&id)
            .or_else(|| old.and_then(|old| old.parents.get(&id)))
            .copied()
            .flatten();
    }
    false
}
impl Tracker {
    pub fn update(
        &mut self,
        current: &BTreeMap<SemanticRef, LiveRegion>,
    ) -> Result<Vec<Announcement>, String> {
        self.previous.retain(|id, _| current.contains_key(id));
        let mut output = Vec::new();
        let mut output_bytes = 0usize;
        for (id, region) in current {
            if region.busy {
                continue;
            }
            let old = self.previous.get(id);
            let old_chunks = old
                .map(|old| {
                    old.chunks
                        .iter()
                        .map(|chunk| (chunk.id, chunk))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            let current_ids = region
                .chunks
                .iter()
                .map(|chunk| chunk.id)
                .collect::<BTreeSet<_>>();
            let mut changes = Vec::new();
            let mut notified = BTreeSet::new();
            let mut baseline = region.clone();
            baseline.chunks.clear();
            for chunk in &region.chunks {
                if deferred(chunk, region, old) {
                    if let Some(previous) = old_chunks.get(&chunk.id) {
                        baseline.chunks.push((*previous).clone());
                        if let Some((group, value)) = previous.group.and_then(|group| {
                            old.and_then(|old| old.groups.get(&group))
                                .map(|value| (group, value))
                        }) {
                            baseline.groups.insert(group, value.clone());
                        }
                    }
                    continue;
                }
                baseline.chunks.push(chunk.clone());
                let changed = match old_chunks.get(&chunk.id) {
                    None => chunk.relevant.additions,
                    Some(previous) => previous.text != chunk.text && chunk.relevant.text,
                };
                if changed && old.is_some() {
                    let key = chunk.group.unwrap_or(chunk.id);
                    if notified.insert(key) {
                        changes.push(
                            chunk
                                .group
                                .and_then(|group| region.groups.get(&group))
                                .unwrap_or(&chunk.text)
                                .clone(),
                        );
                    }
                }
            }
            if let Some(old) = old {
                for chunk in &old.chunks {
                    if current_ids.contains(&chunk.id) {
                        continue;
                    }
                    if deferred(chunk, region, Some(old)) {
                        baseline.chunks.push(chunk.clone());
                        for (key, parent) in &old.parents {
                            baseline.parents.entry(*key).or_insert(*parent);
                        }
                        for (key, value) in &old.groups {
                            baseline.groups.entry(*key).or_insert_with(|| value.clone());
                        }
                        continue;
                    }
                    if chunk.relevant.removals {
                        let key = chunk.group.unwrap_or(chunk.id);
                        if notified.insert(key) {
                            changes.push(
                                chunk
                                    .group
                                    .and_then(|group| old.groups.get(&group))
                                    .unwrap_or(&chunk.text)
                                    .clone(),
                            );
                        }
                    }
                }
            } else if region.initial_alert && region.blocked_groups.is_empty() {
                changes.push(
                    baseline
                        .chunks
                        .iter()
                        .map(|chunk| chunk.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            // A newly observed busy subtree has no baseline. Its first stable contribution
            // is an addition, even when the surrounding live region already existed.
            if !changes.is_empty() {
                let text = if region.atomic {
                    region.text.clone()
                } else {
                    changes.join(" ")
                };
                if text.len() > MAX_TEXT {
                    return Err("live announcement delta budget exceeded".into());
                }
                output_bytes = output_bytes
                    .checked_add(text.len())
                    .ok_or("live announcement output overflow")?;
                if output_bytes > 4 * 1024 * 1024 {
                    return Err("live announcement output budget exceeded".into());
                }
                if !text.is_empty() {
                    output.push(Announcement {
                        text,
                        assertive: region.assertive,
                    });
                }
            }
            let mut retained_bytes = 0usize;
            let mut retained_nodes = 0usize;
            for region in self
                .previous
                .iter()
                .filter(|(key, _)| *key != id)
                .map(|(_, region)| region)
                .chain(std::iter::once(&baseline))
            {
                retained_nodes = retained_nodes
                    .checked_add(region.chunks.len())
                    .and_then(|count| count.checked_add(region.parents.len()))
                    .ok_or("live retained identity overflow")?;
                retained_bytes = region
                    .chunks
                    .iter()
                    .map(|chunk| chunk.text.len())
                    .chain(region.groups.values().map(String::len))
                    .try_fold(retained_bytes, |sum, len| sum.checked_add(len))
                    .ok_or("live retained text overflow")?;
            }
            if retained_nodes > MAX_NODES * 2 || retained_bytes > 8 * 1024 * 1024 {
                return Err("live busy retention budget exceeded".into());
            }
            self.previous.insert(*id, baseline);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn update(
        tracker: &mut Tracker,
        regions: &BTreeMap<SemanticRef, LiveRegion>,
    ) -> Vec<Announcement> {
        tracker.update(regions).unwrap()
    }
    fn id(n: u64) -> SemanticRef {
        SemanticRef {
            page_id: PageId(1),
            document_generation: 1,
            node: n,
        }
    }
    fn region(items: &[(u64, &str)]) -> LiveRegion {
        LiveRegion {
            text: items
                .iter()
                .map(|(_, text)| *text)
                .collect::<Vec<_>>()
                .join(" "),
            chunks: items
                .iter()
                .map(|(n, text)| Chunk {
                    id: id(*n),
                    text: (*text).into(),
                    relevant: Relevant::default(),
                    group: None,
                    busy: false,
                })
                .collect(),
            assertive: false,
            busy: false,
            atomic: false,
            initial_alert: false,
            groups: BTreeMap::new(),
            blocked_groups: BTreeSet::new(),
            parents: BTreeMap::new(),
            busy_nodes: BTreeSet::new(),
        }
    }
    #[test]
    fn busy_accumulates_changes_and_release_announces_once() {
        let mut tracker = Tracker::default();
        assert!(
            update(
                &mut tracker,
                &BTreeMap::from([(id(1), region(&[(2, "old")]))])
            )
            .is_empty()
        );
        let mut next = region(&[(2, "new"), (3, "added")]);
        next.busy = true;
        assert!(update(&mut tracker, &BTreeMap::from([(id(1), next.clone())])).is_empty());
        next.busy = false;
        let values = update(&mut tracker, &BTreeMap::from([(id(1), next.clone())]));
        assert_eq!(values[0].text, "new added");
        assert!(update(&mut tracker, &BTreeMap::from([(id(1), next)])).is_empty());
    }
    #[test]
    fn atomic_and_relevant_use_native_node_identity() {
        let mut tracker = Tracker::default();
        let mut first = region(&[(2, "A"), (3, "B")]);
        for chunk in &mut first.chunks {
            chunk.relevant = Relevant::parse("removals");
        }
        update(&mut tracker, &BTreeMap::from([(id(1), first)]));
        let next = region(&[(3, "B")]);
        assert_eq!(
            update(&mut tracker, &BTreeMap::from([(id(1), next)]))[0].text,
            "A"
        );
        let mut next = region(&[(3, "changed"), (4, "C")]);
        next.atomic = true;
        assert_eq!(
            update(&mut tracker, &BTreeMap::from([(id(1), next)]))[0].text,
            "changed C"
        );
    }
    #[test]
    fn initial_assertive_region_is_silent_but_native_alert_announces() {
        let mut tracker = Tracker::default();
        let mut next = region(&[(2, "Notice")]);
        next.assertive = true;
        assert!(update(&mut tracker, &BTreeMap::from([(id(1), next.clone())])).is_empty());
        next.initial_alert = true;
        assert_eq!(
            update(&mut tracker, &BTreeMap::from([(id(9), next)]))[0].text,
            "Notice"
        );
    }
    #[test]
    fn projection_inherits_busy_and_relevant_and_excludes_nested_off() {
        let template = super::super::tests::fixture()
            .nodes
            .into_values()
            .next()
            .unwrap();
        let make = |n: u64, parent: Option<u64>, children: Vec<u64>, name: &str| {
            let mut node = template.clone();
            node.id = id(n);
            node.parent = parent.map(id);
            node.children = children.into_iter().map(id).collect();
            node.role = "generic".into();
            node.name = name.into();
            node.properties.clear();
            node
        };
        let mut parent = make(1, None, vec![2], "");
        parent.properties.insert("busy".into(), true.into());
        parent.properties.insert("relevant".into(), "text".into());
        let mut root = make(2, Some(1), vec![3, 4], "aggregated duplicate");
        root.properties.insert("live".into(), "polite".into());
        let leaf = make(3, Some(2), vec![], "Visible");
        let mut off = make(4, Some(2), vec![5], "");
        off.properties.insert("live".into(), "off".into());
        let hidden = make(5, Some(4), vec![], "Off");
        let nodes = BTreeMap::from([
            (parent.id, parent),
            (root.id, root),
            (leaf.id, leaf),
            (off.id, off),
            (hidden.id, hidden),
        ]);
        let projection = project(&nodes, std::iter::once(id(2))).unwrap();
        let live = &projection[&id(2)];
        assert!(live.busy);
        assert_eq!(live.text, "Visible");
        assert!(live.chunks[0].relevant.text);
        assert!(!live.chunks[0].relevant.additions);
    }

    #[test]
    fn descendant_busy_removal_waits_without_delaying_unrelated_text() {
        let mut tracker = Tracker::default();
        let mut before = region(&[(2, "remove later"), (3, "old")]);
        before.chunks[0].relevant = Relevant::parse("all");
        before.parents = BTreeMap::from([
            (id(2), Some(id(4))),
            (id(4), Some(id(1))),
            (id(3), Some(id(1))),
        ]);
        update(&mut tracker, &BTreeMap::from([(id(1), before)]));
        let mut during = region(&[(3, "changed")]);
        during.parents = BTreeMap::from([(id(4), Some(id(1))), (id(3), Some(id(1)))]);
        during.busy_nodes.insert(id(4));
        assert_eq!(
            update(&mut tracker, &BTreeMap::from([(id(1), during.clone())]))[0].text,
            "changed"
        );
        during.busy_nodes.clear();
        assert_eq!(
            update(&mut tracker, &BTreeMap::from([(id(1), during.clone())]))[0].text,
            "remove later"
        );
        assert!(update(&mut tracker, &BTreeMap::from([(id(1), during)])).is_empty());
    }
    #[test]
    fn explicit_atomic_false_stops_inheritance_without_removing_group_context() {
        let template = super::super::tests::fixture()
            .nodes
            .into_values()
            .next()
            .unwrap();
        let make = |n: u64, parent: Option<u64>, children: Vec<u64>, name: &str| {
            let mut node = template.clone();
            node.id = id(n);
            node.parent = parent.map(id);
            node.children = children.into_iter().map(id).collect();
            node.role = "generic".into();
            node.name = name.into();
            node.properties.clear();
            node
        };
        let mut root = make(1, None, vec![2, 3], "");
        root.properties.insert("live".into(), "polite".into());
        root.properties.insert("atomic".into(), true.into());
        let a = make(2, Some(1), vec![], "A");
        let mut stop = make(3, Some(1), vec![4], "");
        stop.properties.insert("atomic".into(), false.into());
        let b = make(4, Some(3), vec![], "B");
        let mut nodes = BTreeMap::from([(root.id, root), (a.id, a), (stop.id, stop), (b.id, b)]);
        let mut tracker = Tracker::default();
        update(
            &mut tracker,
            &project(&nodes, std::iter::once(id(1))).unwrap(),
        );
        nodes.get_mut(&id(4)).unwrap().name = "B2".into();
        assert_eq!(
            update(
                &mut tracker,
                &project(&nodes, std::iter::once(id(1))).unwrap()
            )[0]
            .text,
            "B2"
        );
        nodes.get_mut(&id(2)).unwrap().name = "A2".into();
        assert_eq!(
            update(
                &mut tracker,
                &project(&nodes, std::iter::once(id(1))).unwrap()
            )[0]
            .text,
            "A2 B2"
        );
    }
}
