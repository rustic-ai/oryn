use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typedef {
    pub name: String,
    pub type_name: String,
    pub type_descriptor: TypeDescriptor,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

pub(super) fn parse(input: &str) -> Result<Typedef, ParseError> {
    let (extended_attributes, declaration) = take_extended_attributes(input)?;
    let declaration = declaration
        .strip_prefix("typedef ")
        .and_then(|s| s.trim().strip_suffix(';'))
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.into()))?;
    let (ty, name) = declaration
        .rsplit_once(char::is_whitespace)
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.into()))?;
    validate_identifier(name)?;
    Ok(Typedef {
        name: name.into(),
        type_name: ty.trim().into(),
        type_descriptor: parse_type(ty)?,
        extended_attributes,
    })
}

/// Resolve aliases recursively, including references through collections and
/// attributed union members. Reject cycles rather than silently truncating them.
pub fn resolve_type(
    ty: &TypeDescriptor,
    definitions: &[Definition],
) -> Result<TypeDescriptor, ParseError> {
    let mut aliases = BTreeMap::new();
    for definition in definitions {
        if let Definition::Typedef(alias) = definition
            && aliases
                .insert(alias.name.as_str(), &alias.type_descriptor)
                .is_some()
        {
            return Err(ParseError::UnsupportedDefinition(format!(
                "duplicate typedef {}",
                alias.name
            )));
        }
    }
    fn resolve(
        ty: &TypeDescriptor,
        aliases: &BTreeMap<&str, &TypeDescriptor>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<TypeDescriptor, ParseError> {
        use TypeDescriptor::*;
        Ok(match ty {
            Named(name) => {
                if let Some(alias) = aliases.get(name.as_str()) {
                    if !visiting.insert(name.clone()) {
                        return Err(ParseError::UnsupportedDefinition(format!(
                            "cyclic typedef {name}"
                        )));
                    }
                    let result = resolve(alias, aliases, visiting)?;
                    visiting.remove(name);
                    result
                } else {
                    ty.clone()
                }
            }
            Nullable(inner) => Nullable(Box::new(resolve(inner, aliases, visiting)?)),
            Sequence(inner) => Sequence(Box::new(resolve(inner, aliases, visiting)?)),
            Promise(inner) => Promise(Box::new(resolve(inner, aliases, visiting)?)),
            FrozenArray(inner) => FrozenArray(Box::new(resolve(inner, aliases, visiting)?)),
            ObservableArray(inner) => ObservableArray(Box::new(resolve(inner, aliases, visiting)?)),
            Record(key, value) => Record(
                Box::new(resolve(key, aliases, visiting)?),
                Box::new(resolve(value, aliases, visiting)?),
            ),
            Union(members) => Union(
                members
                    .iter()
                    .map(|member| resolve(member, aliases, visiting))
                    .collect::<Result<_, _>>()?,
            ),
            Attributed(attributes, inner) => Attributed(
                attributes.clone(),
                Box::new(resolve(inner, aliases, visiting)?),
            ),
        })
    }
    resolve(ty, &aliases, &mut BTreeSet::new())
}

pub(super) fn emit(definitions: &[Definition], output: &mut String) -> Result<(), ParseError> {
    output.push_str("#[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct GeneratedTypedef { pub name: &'static str, pub type_name: &'static str, pub resolved_type: &'static str }\n#[allow(dead_code)]\npub const GENERATED_TYPEDEFS: &[GeneratedTypedef] = &[\n");
    for definition in definitions {
        if let Definition::Typedef(alias) = definition {
            let resolved = resolve_type(&TypeDescriptor::Named(alias.name.clone()), definitions)?;
            output.push_str(&format!(
                "GeneratedTypedef {{ name: {:?}, type_name: {:?}, resolved_type: {:?} }},\n",
                alias.name,
                alias.type_name,
                format!("{resolved:?}")
            ));
        }
    }
    output.push_str("];\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_preserve_attributed_unions_and_resolve_nested_forward_references() {
        let definitions = parse_definitions("typedef sequence<Later> Values; typedef ([Clamp] unsigned long or DOMString) Later; interface Example { attribute Values values; };").unwrap();
        let TypeDescriptor::Sequence(inner) =
            resolve_type(&TypeDescriptor::Named("Values".into()), &definitions).unwrap()
        else {
            panic!("sequence")
        };
        let TypeDescriptor::Union(members) = *inner else {
            panic!("union")
        };
        assert!(
            matches!(&members[0], TypeDescriptor::Attributed(attrs,_) if attrs[0].name=="Clamp")
        );
        let generated = emit_binding_registry(&definitions).unwrap();
        assert!(generated.contains("GENERATED_TYPEDEFS"));
        assert!(generated.contains("Clamp"));
    }
    #[test]
    fn duplicate_and_cyclic_aliases_are_rejected() {
        for source in [
            "typedef B A; typedef sequence<A> B;",
            "typedef DOMString A; typedef long A;",
        ] {
            assert!(emit_binding_registry(&parse_definitions(source).unwrap()).is_err());
        }
    }
    #[test]
    fn complete_authored_registry_matches_checked_in_generation() {
        let definitions = parse_definitions(include_str!("../idl/browser.webidl")).unwrap();
        assert_eq!(
            emit_binding_registry(&definitions).unwrap(),
            include_str!("../../oryn-js/src/generated/browser.rs")
        );
        let interfaces = merge_partial_interfaces(&definitions).unwrap();
        for name in [
            "Navigation",
            "Performance",
            "Sanitizer",
            "TextTrack",
            "MediaSource",
        ] {
            assert!(interfaces.iter().any(|interface| interface.name == name));
        }
    }
}

#[cfg(test)]
mod static_tests {
    use super::*;
    #[test]
    fn static_attributes_and_operations_keep_constructor_placement_metadata() {
        let definitions = parse_definitions("interface Example { static readonly attribute FrozenArray<DOMString> supported; static boolean supports(DOMString value); readonly attribute boolean active; };").unwrap();
        let generated = emit_binding_registry(&definitions).unwrap();
        assert_eq!(generated.matches("static_member: true").count(), 2);
        assert_eq!(generated.matches("static_member: false").count(), 1);
        assert!(!generated.contains("static boolean"));
    }
}
