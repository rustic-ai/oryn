//! CSS Lists counter identity and value inheritance over the flattened tree.
use super::*;
pub(super) type Key = (u64, &'static str);
pub(super) type Values = BTreeMap<String, Vec<i64>>;
#[derive(Clone)]
struct Counter {
    name: String,
    origin: Key,
    reversed: bool,
    value: i64,
}
#[derive(Clone)]
struct Change {
    name: String,
    value: Option<i64>,
    reversed: bool,
}
struct Event {
    key: Key,
    parent: Option<Key>,
    style: ComputedStyle,
    active: bool,
    list: bool,
}
fn changes(style: &BTreeMap<String, String>, property: &str) -> Result<Vec<Change>, LayoutError> {
    let source = style_value(style, property, "none");
    if source == "none" {
        return Ok(Vec::new());
    }
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut result = Vec::new();
    while !parser.is_exhausted() {
        let (name, reversed) = if let Ok(name) = parser.try_parse(|p| p.expect_ident_cloned()) {
            (name.to_string(), false)
        } else {
            if property != "counter-reset" {
                return Err(LayoutError::StyleTraversal(
                    "invalid counter manipulation".into(),
                ));
            }
            parser
                .expect_function_matching("reversed")
                .map_err(|_| LayoutError::StyleTraversal("invalid reversed counter".into()))?;
            let name = parser
                .parse_nested_block(|p| -> Result<String, cssparser::ParseError<'_, ()>> {
                    let name = p.expect_ident_cloned()?.to_string();
                    p.expect_exhausted()?;
                    Ok(name)
                })
                .map_err(|_| LayoutError::StyleTraversal("invalid reversed counter name".into()))?;
            (name, true)
        };
        result.push(Change {
            name,
            value: parser.try_parse(|p| p.expect_integer()).ok().map(i64::from),
            reversed,
        });
        if result.len() > 4096 {
            return Err(LayoutError::StyleTraversal(
                "counter property exceeds 4096 entries".into(),
            ));
        }
    }
    Ok(result)
}
fn references(source: &str) -> Vec<String> {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut result = Vec::new();
    while let Ok(token) = parser.next() {
        if let cssparser::Token::Function(name) = token {
            if name.eq_ignore_ascii_case("counter") || name.eq_ignore_ascii_case("counters") {
                if let Ok(name) = parser.parse_nested_block(
                    |p| -> Result<String, cssparser::ParseError<'_, ()>> {
                        let name = p.expect_ident_cloned()?.to_string();
                        while !p.is_exhausted() {
                            p.next()?;
                        }
                        Ok(name)
                    },
                ) {
                    result.push(name);
                }
            }
        }
    }
    result
}
fn events(
    nodes: &BTreeMap<u64, LayoutNode>,
    styles: &ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) -> Vec<Event> {
    let empty = Arc::new(BTreeMap::new());
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = nodes
        .values()
        .rev()
        .filter(|node| node.parent.is_none())
        .map(|node| (node.id, "", None))
        .collect::<Vec<_>>();
    while let Some((id, pseudo, parent)) = pending.pop() {
        let Some(node) = nodes.get(&id) else { continue };
        let Some(style) = styles.get(&id) else {
            continue;
        };
        if pseudo != "" {
            let pseudo_style = pseudos.get(&id).and_then(|styles| styles.get(pseudo));
            if pseudo_style.is_none() && pseudo != "marker" {
                continue;
            }
            let pseudo_style = pseudo_style.unwrap_or(&empty);
            let content = style_value(pseudo_style, "content", "normal");
            if style_value(pseudo_style, "display", "inline") == "none"
                || content == "none"
                || (pseudo != "marker" && content == "normal")
            {
                continue;
            }
            result.push(Event {
                key: (id, pseudo),
                parent,
                style: pseudo_style.clone(),
                active: true,
                list: false,
            });
            continue;
        }
        if !visited.insert(id) || style_value(style, "display", "inline") == "none" {
            continue;
        }
        let list = style_value(style, "display", "inline")
            .split_ascii_whitespace()
            .any(|value| value == "list-item");
        let contents = style_value(style, "display", "inline") == "contents";
        if !contents {
            result.push(Event {
                key: (id, ""),
                parent,
                style: style.clone(),
                active: true,
                list,
            });
        }
        let parent = if contents { parent } else { Some((id, "")) };
        if node.tag == "select" && style_value(style, "appearance", "auto") == "base-select" {
            pending.push((id, "picker-icon", parent));
        }
        pending.push((id, "after", parent));
        pending.extend(node.children.iter().rev().map(|id| (*id, "", parent)));
        pending.push((id, "before", parent));
        if node.tag == "option" {
            pending.push((id, "checkmark", parent));
        }
        if list {
            pending.push((id, "marker", parent));
        }
    }
    result
}
#[derive(Default)]
struct Tally {
    sum: i64,
    last: i64,
    stopped: bool,
}
pub(super) fn build(
    nodes: &BTreeMap<u64, LayoutNode>,
    styles: &ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) -> Result<BTreeMap<Key, Values>, LayoutError> {
    let events = events(nodes, styles, pseudos);
    let parents = events
        .iter()
        .map(|event| (event.key, event.parent))
        .collect::<BTreeMap<_, _>>();
    let mut initial = BTreeMap::<(Key, String), i64>::new();
    let mut output = BTreeMap::new();
    for pass in 0..2 {
        let mut states = BTreeMap::<Key, Vec<Counter>>::new();
        let mut siblings = BTreeMap::<Option<Key>, Key>::new();
        let mut previous = Vec::<Counter>::new();
        let mut tallies = BTreeMap::<(Key, String), Tally>::new();
        let mut entries = 0usize;
        for event in &events {
            let source = siblings
                .get(&event.parent)
                .copied()
                .or(event.parent)
                .and_then(|key| states.get(&key));
            let mut counters = if event.parent.is_none() {
                Vec::new()
            } else {
                previous
                    .iter()
                    .filter(|counter| {
                        source.is_some_and(|source| {
                            source.iter().any(|entry| {
                                entry.name == counter.name && entry.origin == counter.origin
                            })
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let instantiate = |counters: &mut Vec<Counter>, change: &Change, value: i64| {
                if let Some(index) = counters
                    .iter()
                    .rposition(|counter| counter.name == change.name)
                {
                    let origin = counters[index].origin;
                    if origin == event.key || parents.get(&origin) == Some(&event.parent) {
                        counters.remove(index);
                    }
                }
                counters.push(Counter {
                    name: change.name.clone(),
                    origin: event.key,
                    reversed: change.reversed,
                    value,
                });
            };
            if event.active {
                for reset in changes(&event.style, "counter-reset")? {
                    let key = (event.key, reset.name.clone());
                    let value = reset.value.unwrap_or_else(|| {
                        if reset.reversed {
                            *initial.get(&key).unwrap_or(&0)
                        } else {
                            0
                        }
                    });
                    instantiate(&mut counters, &reset, value);
                    if pass == 0 && reset.reversed && reset.value.is_none() {
                        tallies.insert(key, Tally::default());
                    }
                }
                let mut increments = changes(&event.style, "counter-increment")?;
                if event.list && !increments.iter().any(|change| change.name == "list-item") {
                    increments.push(Change {
                        name: "list-item".into(),
                        value: Some(
                            if counters
                                .iter()
                                .rev()
                                .find(|counter| counter.name == "list-item")
                                .is_some_and(|counter| counter.reversed)
                            {
                                -1
                            } else {
                                1
                            },
                        ),
                        reversed: false,
                    });
                }
                let sets = changes(&event.style, "counter-set")?;
                let mut effects = BTreeMap::<(Key, String), (i64, Option<i64>)>::new();
                for (set, changes) in [(false, increments), (true, sets)] {
                    for change in changes {
                        if !counters.iter().any(|counter| counter.name == change.name) {
                            instantiate(
                                &mut counters,
                                &Change {
                                    name: change.name.clone(),
                                    value: Some(0),
                                    reversed: false,
                                },
                                0,
                            );
                        }
                        let counter = counters
                            .iter_mut()
                            .rev()
                            .find(|counter| counter.name == change.name)
                            .unwrap();
                        let value = change.value.unwrap_or(if set { 0 } else { 1 });
                        let effect = effects
                            .entry((counter.origin, counter.name.clone()))
                            .or_default();
                        if set {
                            counter.value = value;
                            effect.1 = Some(value);
                        } else {
                            counter.value = counter.value.saturating_add(value);
                            effect.0 = effect.0.saturating_add(value);
                        }
                    }
                }
                if pass == 0 {
                    for (key, (increment, set)) in effects {
                        if let Some(tally) = tallies.get_mut(&key) {
                            if tally.stopped {
                                continue;
                            }
                            let negated = increment.saturating_neg();
                            if negated != 0 {
                                tally.last = negated;
                            }
                            if let Some(value) = set {
                                tally.sum = tally.sum.saturating_add(value);
                                tally.stopped = true;
                            } else {
                                tally.sum = tally.sum.saturating_add(negated);
                            }
                        }
                    }
                }
                for name in references(style_value(&event.style, "content", "normal")) {
                    if !counters.iter().any(|counter| counter.name == name) {
                        instantiate(
                            &mut counters,
                            &Change {
                                name,
                                value: Some(0),
                                reversed: false,
                            },
                            0,
                        );
                    }
                }
            }
            entries = entries.saturating_add(counters.len());
            if entries > 1_048_576 {
                return Err(LayoutError::StyleTraversal(
                    "native counter state exceeds one million entries".into(),
                ));
            }
            if pass == 1 {
                let mut values = Values::new();
                for counter in &counters {
                    values
                        .entry(counter.name.clone())
                        .or_default()
                        .push(counter.value);
                }
                output.insert(event.key, values);
            }
            previous = counters.clone();
            states.insert(event.key, counters);
            siblings.insert(event.parent, event.key);
        }
        if pass == 0 {
            initial = tallies
                .into_iter()
                .map(|(key, tally)| (key, tally.sum.saturating_add(tally.last)))
                .collect();
        }
    }
    Ok(output)
}
