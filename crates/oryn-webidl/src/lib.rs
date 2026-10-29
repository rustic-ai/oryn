//! WebIDL parsing and binding metadata generation for Oryn.
//!
//! The parser stays intentionally small, but represents the binding constructs
//! used by Oryn's generated browser APIs rather than silently flattening them.

pub mod typedefs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Typedef(typedefs::Typedef),
    Interface(Interface),
    InterfaceMixin(Interface),
    Dictionary(Dictionary),
    Callback(Callback),
    Enumeration(Enumeration),
    Includes { target: String, mixin: String },
}

/// WebIDL extended attributes are executable binding metadata. Keeping the
/// original name and optional payload lets the generator enforce exposure,
/// conversion, descriptor and reaction semantics instead of silently erasing
/// them before code generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedAttribute {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enumeration {
    pub name: String,
    pub values: Vec<String>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub callback: bool,
    pub name: String,
    pub inherits: Option<String>,
    pub partial: bool,
    pub members: Vec<Member>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionary {
    pub name: String,
    pub inherits: Option<String>,
    pub partial: bool,
    pub members: Vec<DictionaryMember>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryMember {
    pub name: String,
    pub type_name: String,
    pub type_descriptor: TypeDescriptor,
    pub required: bool,
    pub default: Option<String>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub name: String,
    pub return_type: String,
    pub return_type_descriptor: TypeDescriptor,
    pub arguments: Vec<Argument>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDescriptor {
    Attributed(Vec<ExtendedAttribute>, Box<TypeDescriptor>),
    Named(String),
    Nullable(Box<TypeDescriptor>),
    Sequence(Box<TypeDescriptor>),
    Promise(Box<TypeDescriptor>),
    FrozenArray(Box<TypeDescriptor>),
    ObservableArray(Box<TypeDescriptor>),
    Record(Box<TypeDescriptor>, Box<TypeDescriptor>),
    Union(Vec<TypeDescriptor>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    Attribute {
        name: String,
        type_name: String,
        type_descriptor: TypeDescriptor,
        readonly: bool,
        extended_attributes: Vec<ExtendedAttribute>,
    },
    Operation {
        indexed_getter: bool,
        named_getter: bool,
        name: String,
        return_type: String,
        return_type_descriptor: TypeDescriptor,
        arguments: Vec<Argument>,
        extended_attributes: Vec<ExtendedAttribute>,
    },
    Constructor {
        arguments: Vec<Argument>,
        extended_attributes: Vec<ExtendedAttribute>,
    },
    Constant {
        name: String,
        type_name: String,
        value: String,
        extended_attributes: Vec<ExtendedAttribute>,
    },
    Iterable {
        key: Option<TypeDescriptor>,
        value: TypeDescriptor,
        extended_attributes: Vec<ExtendedAttribute>,
    },
    Setlike {
        value: TypeDescriptor,
        readonly: bool,
        extended_attributes: Vec<ExtendedAttribute>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    pub type_name: String,
    pub type_descriptor: TypeDescriptor,
    pub optional: bool,
    pub variadic: bool,
    pub default: Option<String>,
    pub extended_attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("missing interface declaration")]
    MissingInterface,
    #[error("unterminated interface body")]
    UnterminatedBody,
    #[error("unsupported WebIDL member: {0}")]
    UnsupportedMember(String),
    #[error("invalid WebIDL identifier: {0}")]
    InvalidIdentifier(String),
    #[error("unsupported WebIDL definition: {0}")]
    UnsupportedDefinition(String),
}

include!("setlike_value_type.rs");
pub fn parse_interface(input: &str) -> Result<Interface, ParseError> {
    let source = strip_line_comments(input);
    let (extended_attributes, source) = take_extended_attributes(source.trim())?;
    let callback = source.trim_start().starts_with("callback interface");
    let partial = source.trim_start().starts_with("partial interface");
    let declaration = source
        .split_once("interface")
        .map(|(_, rest)| rest.trim())
        .ok_or(ParseError::MissingInterface)?;
    let (header, body_and_tail) = declaration
        .split_once('{')
        .ok_or(ParseError::MissingInterface)?;
    let body = braced_body(body_and_tail)?;

    let (name, inherits) = if let Some((name, parent)) = header.split_once(':') {
        (name.trim(), Some(parent.trim().to_string()))
    } else {
        (header.trim(), None)
    };
    validate_identifier(name)?;
    if let Some(parent) = &inherits {
        validate_identifier(parent)?;
    }

    let member_sources = split_top_level(body, ';');
    let mut members = member_sources
        .iter()
        .map(|member| member.trim())
        .filter(|member| !member.is_empty())
        .map(parse_member)
        .collect::<Result<Vec<_>, _>>()?;
    let stringifiers = members.iter().filter_map(|member| match member {
        Member::Attribute { type_name, extended_attributes, .. }
            if has_extended_attribute(extended_attributes, "Stringifier") => Some(type_name.clone()),
        _ => None,
    }).collect::<Vec<_>>();
    if stringifiers.len() > 1 || (!stringifiers.is_empty() && members.iter().any(|member|
        matches!(member, Member::Operation { name, .. } if name == "toString"))) {
        return Err(ParseError::UnsupportedMember("conflicting stringifier declarations".into()));
    }
    if let Some(type_name) = stringifiers.first() {
        members.push(parse_member(&format!("{type_name} toString()"))?);
    }
    let setlike_count=members.iter().filter(|member|matches!(member,Member::Setlike{..})).count();
    if setlike_count>1||(setlike_count!=0&&members.iter().any(|member|matches!(member,Member::Iterable{..}))){
        return Err(ParseError::UnsupportedMember("multiple collection declarations".into()));
    }
    let setlike=members.iter().find_map(|member|match member {
        Member::Setlike{value,readonly,extended_attributes}=>Some((value.clone(),*readonly,extended_attributes.clone())),
        _=>None,
    });
    if let Some((value,readonly,attributes))=setlike {
        let value_name=idl_setlike_value_name(&value);
        let mut implied=vec![parse_member("readonly attribute unsigned long size")?,parse_member(&format!("boolean has({value_name} value)"))?];
        if !readonly{implied.extend([parse_member(&format!("{name} add({value_name} value)"))?,parse_member(&format!("boolean delete({value_name} value)"))?,parse_member("undefined clear()")?]);}
        for mut implied in implied{
            let (implied_name,implied_attributes)=match &mut implied{
                Member::Attribute{name,extended_attributes,..}|Member::Operation{name,extended_attributes,..}=>(name.clone(),extended_attributes),
                _=>unreachable!("setlike implicit member"),
            };
            if members.iter().any(|member|matches!(member,Member::Attribute{name,..}|Member::Operation{name,..} if name==&implied_name)){continue;}
            *implied_attributes=attributes.clone();members.push(implied);
        }
    }

    Ok(Interface {
        callback,
        name: name.to_string(),
        inherits,
        partial,
        members,
        extended_attributes,
    })
}

pub fn parse_definitions(input: &str) -> Result<Vec<Definition>, ParseError> {
    split_definitions(&strip_line_comments(input))
        .into_iter()
        .map(|definition| {
            let trimmed = definition.trim();
            let (_, declaration) = take_extended_attributes(trimmed)?;
            if declaration.starts_with("interface mixin ") {
                parse_interface_mixin(trimmed).map(Definition::InterfaceMixin)
            } else if declaration.starts_with("interface ")
                || declaration.starts_with("callback interface ")
                || declaration.starts_with("partial interface ")
            {
                parse_interface(trimmed).map(Definition::Interface)
            } else if declaration.starts_with("dictionary ")
                || declaration.starts_with("partial dictionary ")
            {
                parse_dictionary(trimmed).map(Definition::Dictionary)
            } else if declaration.starts_with("callback ") {
                parse_callback(trimmed).map(Definition::Callback)
            } else if declaration.starts_with("typedef ") {
                typedefs::parse(trimmed).map(Definition::Typedef)
            } else if declaration.starts_with("enum ") {
                parse_enumeration(trimmed).map(Definition::Enumeration)
            } else if declaration.contains(" includes ") {
                parse_includes(trimmed)
            } else {
                Err(ParseError::UnsupportedDefinition(trimmed.to_string()))
            }
        })
        .collect()
}

pub fn merge_partial_interfaces(definitions: &[Definition]) -> Result<Vec<Interface>, ParseError> {
    let mut merged = std::collections::BTreeMap::<String, Interface>::new();
    let mut mixins = std::collections::BTreeMap::<String, Interface>::new();
    for definition in definitions {
        let interface = match definition {
            Definition::InterfaceMixin(interface) => {
                mixins.insert(interface.name.clone(), interface.clone());
                continue;
            }
            Definition::Interface(interface) => interface,
            _ => continue,
        };
        if let Some(existing) = merged.get_mut(&interface.name) {
            if !interface.partial && !existing.partial {
                return Err(ParseError::UnsupportedDefinition(format!(
                    "duplicate non-partial interface {}",
                    interface.name
                )));
            }
            if existing.inherits.is_none() {
                existing.inherits.clone_from(&interface.inherits);
            }
            existing.members.extend(interface.members.clone());
            existing
                .extended_attributes
                .extend(interface.extended_attributes.clone());
            existing.partial = existing.partial && interface.partial;
        } else {
            merged.insert(interface.name.clone(), interface.clone());
        }
    }
    for definition in definitions {
        let Definition::Includes { target, mixin } = definition else {
            continue;
        };
        let members = mixins
            .get(mixin)
            .ok_or_else(|| {
                ParseError::UnsupportedDefinition(format!("unknown interface mixin {mixin}"))
            })?
            .members
            .clone();
        merged
            .get_mut(target)
            .ok_or_else(|| {
                ParseError::UnsupportedDefinition(format!("unknown includes target {target}"))
            })?
            .members
            .extend(members);
    }
    Ok(merged.into_values().collect())
}

pub fn emit_rust_metadata(interface: &Interface) -> String {
    let parent = interface.inherits.as_deref().unwrap_or("");
    let mut output = format!(
        "pub const INTERFACE_NAME: &str = {:?};\npub const PARENT_INTERFACE: &str = {:?};\n",
        interface.name, parent
    );
    output.push_str("pub const MEMBERS: &[&str] = &[\n");
    for member in &interface.members {
        let name = match member {
            Member::Attribute { name, .. }
            | Member::Operation { name, .. }
            | Member::Constant { name, .. } => name,
            Member::Constructor { .. } => "constructor",
            Member::Iterable { .. } => "@@iterator",
            Member::Setlike { .. } => "@@setlike",
        };
        output.push_str(&format!("    {:?},\n", name));
    }
    output.push_str("];\n");
    output
}

fn required_arity(arguments: &[Argument]) -> usize {
    arguments
        .iter()
        .take_while(|argument| {
            !argument.optional && !argument.variadic && argument.default.is_none()
        })
        .count()
}

fn has_extended_attribute(attributes: &[ExtendedAttribute], name: &str) -> bool {
    attributes.iter().any(|attribute| attribute.name == name)
}

fn extended_attribute_value<'a>(
    attributes: &'a [ExtendedAttribute],
    name: &str,
) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value.as_deref())
}

fn exposure_set(attributes: &[ExtendedAttribute]) -> Vec<String> {
    let Some(value) = extended_attribute_value(attributes, "Exposed") else {
        return vec!["Window".into()];
    };
    value
        .trim_matches(|character| character == '(' || character == ')')
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Emit the deterministic, checked-in interface registry consumed by the V8
/// template installer. Behavioral callbacks remain handwritten Rust.
pub fn emit_binding_registry(definitions: &[Definition]) -> Result<String, ParseError> {
    let interfaces = merge_partial_interfaces(definitions)?;
    let mut output = String::from(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub enum GeneratedMemberKind { Attribute, Operation, Constructor, Constant, Iterable, Setlike }\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]\n\
         pub struct GeneratedMemberId(pub u32);\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedMember {\n\
             pub id: GeneratedMemberId,\n\
             pub name: &'static str,\n\
             pub kind: GeneratedMemberKind,\n\
             pub arity: u8,\n\
             pub readonly: bool,\n\
             pub static_member: bool,\n\
             pub indexed_getter: bool,\n\
             pub named_getter: bool,\n\
             pub value_iterable: bool,\n\
             pub constant_value: Option<&'static str>,\n\
             pub type_name: Option<&'static str>,\n\
             pub secure_context: bool,\n\
             pub same_object: bool,\n\
             pub new_object: bool,\n\
             pub ce_reactions: bool,\n\
             pub unscopable: bool,\n\
             pub unforgeable: bool,\n\
             pub replaceable: bool,\n\
             pub exposed: &'static [&'static str],\n\
             pub put_forwards: Option<&'static str>,\n\
             pub reflect_attribute: Option<&'static str>,\n\
             pub legacy_null_to_empty_string: bool,\n\
             pub legacy_lenient_this: bool,\n\
         }\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedInterface {\n\
             pub name: &'static str,\n\
             pub parent: Option<&'static str>,\n\
             pub exposed: &'static [&'static str],\n\
             pub global_names: &'static [&'static str],\n\
             pub secure_context: bool,\n\
             pub constructible: bool,\n\
             pub callback: bool,\n\
             pub members: &'static [GeneratedMember],\n\
         }\n\n",
    );
    output.push_str(
        "#[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedDictionaryMember { pub name: &'static str, pub type_name: &'static str, pub required: bool, pub default: Option<&'static str> }\n\
         #[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedDictionary { pub name: &'static str, pub parent: Option<&'static str>, pub members: &'static [GeneratedDictionaryMember] }\n\
         #[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedArgument { pub name: &'static str, pub type_name: &'static str, pub optional: bool, pub variadic: bool, pub default: Option<&'static str> }\n\
         #[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedCallback { pub name: &'static str, pub return_type: &'static str, pub arguments: &'static [GeneratedArgument] }\n\
         #[allow(dead_code)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct GeneratedEnum { pub name: &'static str, pub values: &'static [&'static str] }\n\n",
    );
    for (interface_index, interface) in interfaces.iter().enumerate() {
        output.push_str(&format!(
            "pub const {}_MEMBERS: &[GeneratedMember] = &[",
            rust_constant(&interface.name)
        ));
        for (member_index, member) in interface.members.iter().enumerate() {
            let (name, kind, arity, readonly, constant_value, type_name, attributes) = match member
            {
                Member::Attribute {
                    name,
                    readonly,
                    type_name,
                    extended_attributes,
                    ..
                } => (
                    name.as_str(),
                    "Attribute",
                    0,
                    *readonly,
                    None,
                    Some(type_name.as_str()),
                    extended_attributes,
                ),
                Member::Operation {
                    name,
                    arguments,
                    return_type,
                    extended_attributes,
                    ..
                } => (
                    name.as_str(),
                    "Operation",
                    required_arity(arguments),
                    false,
                    None,
                    Some(return_type.as_str()),
                    extended_attributes,
                ),
                Member::Constant {
                    name,
                    value,
                    type_name,
                    extended_attributes,
                    ..
                } => (
                    name.as_str(),
                    "Constant",
                    0,
                    true,
                    Some(value.as_str()),
                    Some(type_name.as_str()),
                    extended_attributes,
                ),
                Member::Constructor {
                    arguments,
                    extended_attributes,
                } => (
                    "constructor",
                    "Constructor",
                    required_arity(arguments),
                    false,
                    None,
                    None,
                    extended_attributes,
                ),
                Member::Iterable {
                    extended_attributes,
                    ..
                } => (
                    "@@iterator",
                    "Iterable",
                    0,
                    true,
                    None,
                    None,
                    extended_attributes,
                ),
                Member::Setlike { readonly, extended_attributes, .. } => (
                    "@@setlike", "Setlike", 0, *readonly, None, None, extended_attributes,
                ),
            };
            let indexed_getter = matches!(
                member,
                Member::Operation {
                    indexed_getter: true,
                    ..
                }
            );
            let named_getter = matches!(
                member,
                Member::Operation {
                    named_getter: true,
                    ..
                }
            );
            let value_iterable = matches!(member, Member::Iterable { key: None, .. });
            let static_member = has_extended_attribute(attributes, "Static");
            let secure_context = has_extended_attribute(attributes, "SecureContext");
            let same_object = has_extended_attribute(attributes, "SameObject");
            let new_object = has_extended_attribute(attributes, "NewObject");
            let ce_reactions = has_extended_attribute(attributes, "CEReactions");
            let unscopable = has_extended_attribute(attributes, "Unscopable");
            let unforgeable = has_extended_attribute(attributes, "LegacyUnforgeable")
                || has_extended_attribute(attributes, "Unforgeable");
            let replaceable = has_extended_attribute(attributes, "Replaceable");
            let put_forwards = extended_attribute_value(attributes, "PutForwards");
            let reflect_attribute = has_extended_attribute(attributes, "Reflect").then(|| {
                extended_attribute_value(attributes, "Reflect")
                    .unwrap_or(name)
                    .to_ascii_lowercase()
            });
            let legacy_null_to_empty_string =
                has_extended_attribute(attributes, "LegacyNullToEmptyString");
            let legacy_lenient_this = has_extended_attribute(attributes, "LegacyLenientThis");
            let exposed = if has_extended_attribute(attributes, "Exposed") {
                exposure_set(attributes)
            } else {
                Vec::new()
            };
            output.push_str(&format!(
                "GeneratedMember {{ id: GeneratedMemberId({token}), name: {name:?}, kind: GeneratedMemberKind::{kind}, arity: {arity}, readonly: {readonly}, static_member: {static_member}, indexed_getter: {indexed_getter}, named_getter: {named_getter}, value_iterable: {value_iterable}, constant_value: {constant_value:?}, type_name: {type_name:?}, secure_context: {secure_context}, same_object: {same_object}, new_object: {new_object}, ce_reactions: {ce_reactions}, unscopable: {unscopable}, unforgeable: {unforgeable}, replaceable: {replaceable}, exposed: &{exposed:?}, put_forwards: {put_forwards:?}, reflect_attribute: {reflect_attribute:?}, legacy_null_to_empty_string: {legacy_null_to_empty_string}, legacy_lenient_this: {legacy_lenient_this} }},",
                token = ((interface_index as u32) << 16) | member_index as u32,
            ));
        }
        output.push_str("];\n");
    }
    output.push_str("\npub const GENERATED_INTERFACES: &[GeneratedInterface] = &[\n");
    for interface in &interfaces {
        let exposed = exposure_set(&interface.extended_attributes);
        let constructible = interface
            .members
            .iter()
            .any(|member| matches!(member, Member::Constructor { .. }))
            || has_extended_attribute(&interface.extended_attributes, "Constructor");
        output.push_str(&format!(
            "    GeneratedInterface {{ name: {:?}, parent: {}, exposed: &{:?}, global_names: &{:?}, secure_context: {}, constructible: {}, callback: {}, members: {}_MEMBERS }},\n",
            interface.name,
            interface
                .inherits
                .as_ref()
                .map(|parent| format!("Some({parent:?})"))
                .unwrap_or_else(|| "None".into()),
            exposed,
            extended_attribute_value(&interface.extended_attributes, "Global")
                .map(|value| value.trim_matches(|c| c == '(' || c == ')')
                    .split(',').map(str::trim).filter(|value| !value.is_empty())
                    .map(str::to_string).collect::<Vec<_>>()).unwrap_or_default(),
            has_extended_attribute(&interface.extended_attributes, "SecureContext"),
            constructible,
            interface.callback,
            rust_constant(&interface.name)
        ));
    }
    output.push_str("];\n\n");
    let dictionaries = definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Dictionary(dictionary) => Some(dictionary),
            _ => None,
        })
        .collect::<Vec<_>>();
    for dictionary in &dictionaries {
        output.push_str(&format!(
            "#[allow(dead_code)] pub const {}_DICTIONARY_MEMBERS: &[GeneratedDictionaryMember] = &[",
            rust_constant(&dictionary.name)
        ));
        for member in &dictionary.members {
            output.push_str(&format!(
                "GeneratedDictionaryMember {{ name: {:?}, type_name: {:?}, required: {}, default: {:?} }},",
                member.name, member.type_name, member.required, member.default
            ));
        }
        output.push_str("];\n");
    }
    output.push_str(
        "#[allow(dead_code)] pub const GENERATED_DICTIONARIES: &[GeneratedDictionary] = &[\n",
    );
    for dictionary in &dictionaries {
        output.push_str(&format!(
            "GeneratedDictionary {{ name: {:?}, parent: {}, members: {}_DICTIONARY_MEMBERS }},\n",
            dictionary.name,
            dictionary
                .inherits
                .as_ref()
                .map(|parent| format!("Some({parent:?})"))
                .unwrap_or_else(|| "None".into()),
            rust_constant(&dictionary.name)
        ));
    }
    output.push_str("];\n\n");
    let callbacks = definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Callback(callback) => Some(callback),
            _ => None,
        })
        .collect::<Vec<_>>();
    for callback in &callbacks {
        output.push_str(&format!(
            "#[allow(dead_code)] pub const {}_CALLBACK_ARGUMENTS: &[GeneratedArgument] = &[",
            rust_constant(&callback.name)
        ));
        for argument in &callback.arguments {
            output.push_str(&format!(
                "GeneratedArgument {{ name: {:?}, type_name: {:?}, optional: {}, variadic: {}, default: {:?} }},",
                argument.name, argument.type_name, argument.optional, argument.variadic, argument.default
            ));
        }
        output.push_str("];\n");
    }
    output
        .push_str("#[allow(dead_code)] pub const GENERATED_CALLBACKS: &[GeneratedCallback] = &[\n");
    for callback in &callbacks {
        output.push_str(&format!(
            "GeneratedCallback {{ name: {:?}, return_type: {:?}, arguments: {}_CALLBACK_ARGUMENTS }},\n",
            callback.name,
            callback.return_type,
            rust_constant(&callback.name)
        ));
    }
    output.push_str("];\n\n");
    output.push_str("#[allow(dead_code)] pub const GENERATED_ENUMS: &[GeneratedEnum] = &[\n");
    for enumeration in definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Enumeration(enumeration) => Some(enumeration),
            _ => None,
        })
    {
        output.push_str(&format!(
            "GeneratedEnum {{ name: {:?}, values: &{:?} }},\n",
            enumeration.name, enumeration.values
        ));
    }
    output.push_str("];\n\n");
    output.push_str(
        "pub fn generated_interface(name: &str) -> Option<&'static GeneratedInterface> {\n\
             GENERATED_INTERFACES.iter().find(|interface| interface.name == name)\n\
         }\n\n\
         pub fn generated_member(interface: &str, member: &str) -> Option<&'static GeneratedMember> {\n\
             generated_interface(interface)?.members.iter().find(|candidate| candidate.name == member)\n\
         }\n\n\
         #[allow(dead_code)]\n\
         pub fn generated_member_by_id(id: GeneratedMemberId) -> Option<(&'static GeneratedInterface, &'static GeneratedMember)> {\n\
             let interface = GENERATED_INTERFACES.get((id.0 >> 16) as usize)?;\n\
             let member = interface.members.get((id.0 & 0xffff) as usize)?;\n\
             (member.id == id).then_some((interface, member))\n\
         }\n",
    );
    typedefs::emit(definitions, &mut output)?;
    let syntax = syn::parse_file(&output).map_err(|error| {
        ParseError::UnsupportedDefinition(format!("generated Rust did not parse: {error}"))
    })?;
    Ok(format!(
        "// @generated by oryn-webidl; do not edit.\n{}",
        prettyplease::unparse(&syntax)
    ))
}

fn parse_member(member: &str) -> Result<Member, ParseError> {
    let (mut extended_attributes, member) = take_extended_attributes(member.trim())?;
    let member = if let Some(rest) = member.strip_prefix("stringifier ") {
        let attribute = rest.strip_prefix("readonly ").unwrap_or(rest);
        if !attribute.starts_with("attribute DOMString ")
            && !attribute.starts_with("attribute USVString ") {
            return Err(ParseError::UnsupportedMember(member.into()));
        }
        extended_attributes.push(ExtendedAttribute { name: "Stringifier".into(), value: None });
        rest
    } else { member };
    let member = if let Some(rest) = member.strip_prefix("static ") {
        extended_attributes.push(ExtendedAttribute {
            name: "Static".into(),
            value: None,
        });
        rest.trim()
    } else {
        member
    };
    let (readonly, member) = member
        .strip_prefix("readonly ")
        .map_or((false, member), |rest| (true, rest.trim()));

    if let Some(value) = member.strip_prefix("setlike<").and_then(|rest|rest.strip_suffix('>')) {
        if value.trim().is_empty()||split_top_level(value, ',').len()!=1
            ||has_extended_attribute(&extended_attributes,"Static") {
            return Err(ParseError::UnsupportedMember(member.into()));
        }
        return Ok(Member::Setlike { value:parse_type(value.trim())?, readonly, extended_attributes });
    }

    if let Some(arguments) = member
        .strip_prefix("constructor(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Ok(Member::Constructor {
            arguments: parse_arguments(arguments)?,
            extended_attributes,
        });
    }

    if let Some(value) = member.strip_prefix("const ")
        && let Some((declaration, constant)) = value.split_once('=')
        && let Some((type_name, name)) = split_type_and_name(declaration.trim())
    {
        validate_identifier(name)?;
        parse_type(type_name)?;
        return Ok(Member::Constant {
            name: name.into(),
            type_name: type_name.into(),
            value: constant.trim().into(),
            extended_attributes,
        });
    }

    if let Some(types) = member
        .strip_prefix("iterable<")
        .and_then(|value| value.strip_suffix('>'))
    {
        let types = split_top_level(types, ',');
        return match types.as_slice() {
            [value] => Ok(Member::Iterable {
                key: None,
                value: parse_type(value)?,
                extended_attributes,
            }),
            [key, value] => Ok(Member::Iterable {
                key: Some(parse_type(key)?),
                value: parse_type(value)?,
                extended_attributes,
            }),
            _ => Err(ParseError::UnsupportedMember(member.into())),
        };
    }

    if let Some(attribute) = member.strip_prefix("attribute ")
        && let Some((type_name, name)) = split_type_and_name(attribute)
    {
        validate_identifier(name)?;
        return Ok(Member::Attribute {
            name: name.to_string(),
            type_name: type_name.to_string(),
            type_descriptor: parse_type(type_name)?,
            readonly,
            extended_attributes,
        });
    }

    let (special, member) = member
        .split_once(char::is_whitespace)
        .filter(|(keyword, _)| matches!(*keyword, "getter" | "setter" | "deleter"))
        .map_or((None, member), |(keyword, member)| {
            (Some(keyword), member.trim())
        });
    if !readonly
        && let Some((head, arguments)) = operation_signature(member)
        && let Some((return_type, name)) = if special.is_some() && anonymous_operation_head(head) {
            Some((head.trim(), ""))
        } else {
            split_type_and_name(head)
        }
    {
        if !name.is_empty() {
            validate_identifier(name)?;
        }
        let arguments = parse_arguments(arguments)?;
        if let Some(special) = special
            && (arguments.len() != (if special == "setter" { 2 } else { 1 })
                || !matches!(
                    arguments[0].type_name.as_str(),
                    "unsigned long" | "DOMString"
                )
                || (special == "deleter" && arguments[0].type_name != "DOMString")
                || arguments.iter().any(|argument| {
                    argument.optional || argument.variadic || argument.default.is_some()
                })
                || (special == "setter" && return_type != "undefined"))
        {
            return Err(ParseError::UnsupportedMember(member.into()));
        }
        return Ok(Member::Operation {
            indexed_getter: special == Some("getter") && arguments[0].type_name == "unsigned long",
            named_getter: special == Some("getter") && arguments[0].type_name == "DOMString",
            name: name.to_string(),
            return_type: return_type.to_string(),
            return_type_descriptor: parse_type(return_type)?,
            arguments,
            extended_attributes,
        });
    }

    Err(ParseError::UnsupportedMember(member.to_string()))
}

// A special operation may omit its identifier. Only a complete type is an
// anonymous head: a named operation such as `DOMString item` must still split.
fn anonymous_operation_head(head: &str) -> bool {
    fn complete_type(descriptor: &TypeDescriptor) -> bool {
        match descriptor {
            TypeDescriptor::Named(name) => {
                validate_identifier(name).is_ok()
                    || matches!(
                        name.as_str(),
                        "unsigned short"
                            | "unsigned long"
                            | "long long"
                            | "unsigned long long"
                            | "unrestricted float"
                            | "unrestricted double"
                    )
            }
            TypeDescriptor::Attributed(_, inner)
            | TypeDescriptor::Nullable(inner)
            | TypeDescriptor::Sequence(inner)
            | TypeDescriptor::Promise(inner)
            | TypeDescriptor::FrozenArray(inner)
            | TypeDescriptor::ObservableArray(inner) => complete_type(inner),
            TypeDescriptor::Record(key, value) => complete_type(key) && complete_type(value),
            TypeDescriptor::Union(members) => members.iter().all(complete_type),
        }
    }
    parse_type(head).is_ok_and(|descriptor| complete_type(&descriptor))
}

fn parse_arguments(arguments: &str) -> Result<Vec<Argument>, ParseError> {
    if arguments.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parts = split_top_level(arguments, ',');
    let count = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(index, argument)| {
            let argument = argument.trim();
            let (extended_attributes, argument) = take_extended_attributes(argument)?;
            let (optional, argument) = argument
                .strip_prefix("optional ")
                .map_or((false, argument), |value| (true, value.trim()));
            let (argument, default) = argument
                .split_once('=')
                .map_or((argument, None), |(value, default)| {
                    (value.trim(), Some(default.trim().to_string()))
                });
            if let Some((type_name, name)) = split_type_and_name(argument) {
                validate_identifier(name)?;
                let (type_name, variadic) = type_name
                    .strip_suffix("...")
                    .map_or((type_name, false), |value| (value.trim(), true));
                if variadic && (optional || default.is_some() || index + 1 != count) {
                    return Err(ParseError::UnsupportedMember(argument.to_string()));
                }
                Ok(Argument {
                    name: name.to_string(),
                    type_name: type_name.to_string(),
                    type_descriptor: parse_type(type_name)?,
                    optional,
                    variadic,
                    default,
                    extended_attributes,
                })
            } else {
                Err(ParseError::UnsupportedMember(argument.trim().to_string()))
            }
        })
        .collect()
}

fn parse_dictionary(input: &str) -> Result<Dictionary, ParseError> {
    let (extended_attributes, input) = take_extended_attributes(input.trim())?;
    let partial = input.trim_start().starts_with("partial dictionary");
    let declaration = input
        .split_once("dictionary")
        .map(|(_, rest)| rest.trim())
        .ok_or(ParseError::MissingInterface)?;
    let (header, body_and_tail) = declaration
        .split_once('{')
        .ok_or(ParseError::MissingInterface)?;
    let body = braced_body(body_and_tail)?;
    let (name, inherits) = if let Some((name, parent)) = header.split_once(':') {
        (name.trim(), Some(parent.trim().to_string()))
    } else {
        (header.trim(), None)
    };
    validate_identifier(name)?;
    if let Some(parent) = &inherits {
        validate_identifier(parent)?;
    }
    let member_sources = split_top_level(body, ';');
    let members = member_sources
        .iter()
        .map(|member| member.trim())
        .filter(|member| !member.is_empty())
        .map(|member| {
            let (extended_attributes, member) = take_extended_attributes(member)?;
            let (required, member) = member
                .strip_prefix("required ")
                .map_or((false, member), |value| (true, value.trim()));
            let (member, default) = member
                .split_once('=')
                .map_or((member, None), |(value, default)| {
                    (value.trim(), Some(default.trim().to_string()))
                });
            let (type_name, name) = split_type_and_name(member)
                .ok_or_else(|| ParseError::UnsupportedMember(member.to_string()))?;
            validate_identifier(name)?;
            Ok(DictionaryMember {
                name: name.to_string(),
                type_name: type_name.to_string(),
                type_descriptor: parse_type(type_name)?,
                required,
                default,
                extended_attributes,
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Dictionary {
        name: name.to_string(),
        inherits,
        partial,
        members,
        extended_attributes,
    })
}

fn parse_callback(input: &str) -> Result<Callback, ParseError> {
    let (extended_attributes, input) = take_extended_attributes(input.trim())?;
    let declaration = input
        .trim()
        .strip_prefix("callback ")
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.to_string()))?;
    let (name, signature) = declaration
        .split_once('=')
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.to_string()))?;
    let name = name.trim();
    validate_identifier(name)?;
    let signature = signature.trim().trim_end_matches(';').trim();
    let (head, arguments) = signature
        .split_once('(')
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.to_string()))?;
    let arguments = arguments
        .strip_suffix(')')
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.to_string()))?;
    Ok(Callback {
        name: name.to_string(),
        return_type: head.trim().to_string(),
        return_type_descriptor: parse_type(head.trim())?,
        arguments: parse_arguments(arguments)?,
        extended_attributes,
    })
}

fn parse_type(input: &str) -> Result<TypeDescriptor, ParseError> {
    let input = input.trim();
    if input.starts_with('[') {
        let (attributes, rest) = take_extended_attributes(input)?;
        return Ok(TypeDescriptor::Attributed(
            attributes,
            Box::new(parse_type(rest)?),
        ));
    }
    if let Some(inner) = input.strip_suffix('?') {
        return Ok(TypeDescriptor::Nullable(Box::new(parse_type(inner)?)));
    }
    if let Some(inner) = input
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    {
        let members = inner
            .split(" or ")
            .map(parse_type)
            .collect::<Result<Vec<_>, _>>()?;
        if members.len() < 2 {
            return Err(ParseError::UnsupportedMember(input.into()));
        }
        return Ok(TypeDescriptor::Union(members));
    }
    if let Some(inner) = input
        .strip_prefix("record<")
        .and_then(|value| value.strip_suffix('>'))
    {
        let members = split_top_level(inner, ',');
        if let [key, value] = members.as_slice() {
            return Ok(TypeDescriptor::Record(
                Box::new(parse_type(key)?),
                Box::new(parse_type(value)?),
            ));
        }
        return Err(ParseError::UnsupportedMember(input.into()));
    }
    for (prefix, constructor) in [
        (
            "sequence<",
            TypeDescriptor::Sequence as fn(Box<TypeDescriptor>) -> TypeDescriptor,
        ),
        ("Promise<", TypeDescriptor::Promise),
        ("FrozenArray<", TypeDescriptor::FrozenArray),
        ("ObservableArray<", TypeDescriptor::ObservableArray),
    ] {
        if let Some(inner) = input
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix('>'))
        {
            return Ok(constructor(Box::new(parse_type(inner)?)));
        }
    }
    if input.is_empty()
        || !input.split_whitespace().all(|part| {
            part.chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric())
        })
    {
        return Err(ParseError::UnsupportedMember(input.to_string()));
    }
    Ok(TypeDescriptor::Named(input.to_string()))
}

fn parse_interface_mixin(input: &str) -> Result<Interface, ParseError> {
    parse_interface(&input.replacen("interface mixin", "interface", 1))
}

fn parse_enumeration(input: &str) -> Result<Enumeration, ParseError> {
    let (extended_attributes, input) = take_extended_attributes(input.trim())?;
    let declaration = input
        .trim()
        .strip_prefix("enum ")
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.into()))?;
    let (name, body) = declaration
        .split_once('{')
        .ok_or(ParseError::UnterminatedBody)?;
    let body = braced_body(body)?;
    let name = name.trim();
    validate_identifier(name)?;
    let values = split_top_level(body, ',')
        .into_iter()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
        .collect();
    Ok(Enumeration {
        name: name.into(),
        values,
        extended_attributes,
    })
}

fn parse_includes(input: &str) -> Result<Definition, ParseError> {
    let value = input.trim().trim_end_matches(';');
    let (target, mixin) = value
        .split_once(" includes ")
        .ok_or_else(|| ParseError::UnsupportedDefinition(input.into()))?;
    validate_identifier(target.trim())?;
    validate_identifier(mixin.trim())?;
    Ok(Definition::Includes {
        target: target.trim().into(),
        mixin: mixin.trim().into(),
    })
}

// Find the operation's argument list without confusing a union return type
// or a generic type's parentheses with that list.
fn operation_signature(input: &str) -> Option<(&str, &str)> {
    let mut parentheses = 0usize;
    let mut angles = 0usize;
    for (index, character) in input.char_indices() {
        match character {
            '<' => angles += 1,
            '>' => angles = angles.saturating_sub(1),
            '(' if parentheses == 0 && angles == 0 && index > 0 => {
                return Some((input[..index].trim(), input[index + 1..].strip_suffix(')')?));
            }
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.checked_sub(1)?,
            _ => {}
        }
    }
    None
}

fn split_type_and_name(input: &str) -> Option<(&str, &str)> {
    let index = input.rfind(char::is_whitespace)?;
    let type_name = input[..index].trim();
    let name = input[index..].trim();
    (!type_name.is_empty() && !name.is_empty()).then_some((type_name, name))
}

// Input starts just after the opening brace. Dictionary defaults may contain
// their own braces and quoted strings may contain arbitrary delimiters.
fn braced_body(input: &str) -> Result<&str, ParseError> {
    let mut depth = 1;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&input[..index]);
                }
            }
            _ => {}
        }
    }
    Err(ParseError::UnterminatedBody)
}

fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut depth = 0_i32;
    let mut quoted = None;
    let mut escaped = false;
    let mut start = 0;
    let mut values = Vec::new();
    for (index, character) in input.char_indices() {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quoted = Some(character),
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            value if value == separator && depth == 0 => {
                values.push(input[start..index].to_string());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    values.push(input[start..].to_string());
    values
}

fn split_definitions(input: &str) -> Vec<String> {
    split_top_level(input, ';')
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{value};"))
        .collect()
}

fn validate_identifier(value: &str) -> Result<(), ParseError> {
    let valid = value.chars().enumerate().all(|(index, character)| {
        character == '_'
            || (index > 0 && character == '-')
            || character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit())
    });
    if value.is_empty() || !valid {
        return Err(ParseError::InvalidIdentifier(value.to_string()));
    }
    Ok(())
}

fn strip_line_comments(input: &str) -> String {
    let mut output = String::new();
    let mut characters = input.chars().peekable();
    let mut quote = None;
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if let Some(delimiter) = quote {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
            output.push(character);
        } else if character == '/' && characters.peek() == Some(&'/') {
            characters.next();
            for next in characters.by_ref() {
                if next == '\n' {
                    output.push('\n');
                    break;
                }
            }
        } else if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            output.push(' ');
            while let Some(next) = characters.next() {
                if next == '*' && characters.peek() == Some(&'/') {
                    characters.next();
                    break;
                }
                if next == '\n' {
                    output.push('\n');
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn take_extended_attributes(input: &str) -> Result<(Vec<ExtendedAttribute>, &str), ParseError> {
    let input = input.trim_start();
    if !input.starts_with('[') {
        return Ok((Vec::new(), input));
    }
    let mut depth = 0_u32;
    let mut quoted = None;
    let mut end = None;
    for (index, character) in input.char_indices() {
        if let Some(quote) = quoted {
            if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quoted = Some(character),
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| ParseError::UnsupportedDefinition(input.to_string()))?;
    let payload = &input[1..end];
    let attributes = split_top_level(payload, ',')
        .into_iter()
        .map(|attribute| {
            let attribute = attribute.trim();
            let (name, value) = if let Some((name, value)) = attribute.split_once('=') {
                (name.trim(), Some(value.trim().to_string()))
            } else if let Some((name, value)) = attribute.split_once('(') {
                (
                    name.trim(),
                    Some(value.trim_end_matches(')').trim().to_string()),
                )
            } else {
                (attribute, None)
            };
            validate_identifier(name)?;
            Ok(ExtendedAttribute {
                name: name.to_string(),
                value,
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok((attributes, input[end + 1..].trim_start()))
}

fn rust_constant(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            output.push('_');
        }
        output.push(character.to_ascii_uppercase());
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn stringifier_attributes_generate_native_operation_without_reaction_forwarding() {
        let interface = super::parse_interface("interface Link { [CEReactions] stringifier attribute USVString href; };").unwrap();
        assert!(matches!(&interface.members[0],super::Member::Attribute { name, extended_attributes, .. }
            if name=="href" && super::has_extended_attribute(extended_attributes,"CEReactions")
                && super::has_extended_attribute(extended_attributes,"Stringifier")));
        assert!(matches!(&interface.members[1],super::Member::Operation { name, return_type, arguments, extended_attributes, .. }
            if name=="toString" && return_type=="USVString" && arguments.is_empty() && extended_attributes.is_empty()));
        for input in ["interface Bad { stringifier attribute long value; };",
            "interface Bad { stringifier attribute DOMString first; stringifier attribute DOMString second; };",
            "interface Bad { stringifier attribute USVString href; DOMString toString(); };"] {
            assert!(super::parse_interface(input).is_err());
        }
    }
    include!("setlike_controls.rs");
    use super::*;

    #[test]
    fn global_metadata_is_distinct_from_exposure_and_inherited_mixin_members() {
        let definitions = parse_definitions(r#"
          [Global=Window,Exposed=Window] interface Window {};
          [Exposed=Worker] interface WorkerGlobalScope {};
          [Global=(Worker,DedicatedWorker),Exposed=DedicatedWorker]
          interface DedicatedWorkerGlobalScope : WorkerGlobalScope {};
          interface mixin Globals { [SameObject] readonly attribute Factory indexedDB; };
          Window includes Globals; WorkerGlobalScope includes Globals;
        "#).unwrap();
        let registry = emit_binding_registry(&definitions).unwrap();
        assert!(registry.contains(r#"global_names: &["Window"]"#));
        assert!(registry.contains(r#"global_names: &["Worker", "DedicatedWorker"]"#));
        // The emitted registry is pretty-printed. Check the full field sequence
        // without requiring it to fit on a single physical line.
        let compact: String = registry.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains(r#"name:"WorkerGlobalScope",parent:None,exposed:&["Worker"],global_names:&[],secure_context:false"#));
        assert!(registry.contains(r#"name: "indexedDB""#));
    }


    #[test]
    fn nested_dictionary_defaults_do_not_end_interface_or_dictionary_bodies() {
        let definitions = parse_definitions(
            r#"
            interface Example { constructor(optional Options options = {}); undefined run(); };
            dictionary Options { Options nested = {}; DOMString url = "https://example.test/};"; };
        "#,
        )
        .unwrap();
        let Definition::Interface(interface) = &definitions[0] else {
            panic!("interface")
        };
        assert_eq!(interface.members.len(), 2);
        let Member::Constructor { arguments, .. } = &interface.members[0] else {
            panic!("constructor")
        };
        assert_eq!(arguments[0].default.as_deref(), Some("{}"));
        let Definition::Dictionary(dictionary) = &definitions[1] else {
            panic!("dictionary")
        };
        assert_eq!(dictionary.members.len(), 2);
        assert_eq!(dictionary.members[0].default.as_deref(), Some("{}"));
        assert_eq!(
            dictionary.members[1].default.as_deref(),
            Some("\"https://example.test/};\"")
        );
    }

    #[test]
    fn lexical_delimiters_in_strings_and_comments_do_not_split_definitions() {
        let source = r#"/* ignored }; */ enum Values { "}", "a;b", "https://example.test/" }; // trailing
            interface Next { constructor(); };
        "#;
        let definitions = parse_definitions(source).unwrap();
        assert_eq!(definitions.len(), 2);
        let Definition::Enumeration(values) = &definitions[0] else {
            panic!("enum")
        };
        assert_eq!(values.values, ["}", "a;b", "https://example.test/"]);
        assert_eq!(braced_body("{}};"), Ok("{}"));
        assert_eq!(braced_body("{}"), Err(ParseError::UnterminatedBody));
    }

    #[test]
    fn hyphenated_css_attributes_preserve_their_webidl_names() {
        let definitions = parse_definitions("[Exposed=Window] interface CSSPageDescriptors { attribute DOMString margin-top; attribute DOMString page-orientation; };").unwrap();
        let generated = emit_binding_registry(&definitions).unwrap();
        assert!(generated.contains("name: \"margin-top\""));
        assert!(generated.contains("name: \"page-orientation\""));
        assert!(parse_definitions("interface -Invalid { };").is_err());
    }

    #[test]
    fn anonymous_indexed_getters_preserve_metadata_without_a_method_name() {
        let definitions = parse_definitions("[Exposed=Window] interface HTMLAllCollection { getter Element (unsigned long index); getter (HTMLCollection or Element)? namedItem(DOMString name); (HTMLCollection or Element)? item(optional DOMString nameOrIndex); };").unwrap();
        let interfaces = merge_partial_interfaces(&definitions).unwrap();
        assert!(
            matches!(&interfaces[0].members[0], Member::Operation { name, indexed_getter: true, .. } if name.is_empty())
        );
        assert!(matches!(
            &interfaces[0].members[1],
            Member::Operation {
                named_getter: true,
                return_type_descriptor: TypeDescriptor::Nullable(_),
                ..
            }
        ));
        let generated = emit_binding_registry(&definitions).unwrap();
        assert!(generated.contains("name: \"\""));
        assert!(generated.contains("indexed_getter: true"));
    }

    #[test]
    fn anonymous_special_operations_preserve_types_arity_and_reactions() {
        let definitions = parse_definitions(
            r#"
            [Exposed=Window] interface DOMStringMap {
                getter DOMString (DOMString name);
                [CEReactions] setter undefined (DOMString name, DOMString value);
                [CEReactions] deleter undefined (DOMString name);
            };
        "#,
        )
        .unwrap();
        let Definition::Interface(interface) = &definitions[0] else {
            panic!("interface")
        };
        assert_eq!(interface.members.len(), 3);
        for (member, (expected_type, arity, getter, reactions)) in interface.members.iter().zip([
            ("DOMString", 1, true, false),
            ("undefined", 2, false, true),
            ("undefined", 1, false, true),
        ]) {
            let Member::Operation {
                name,
                return_type,
                return_type_descriptor,
                arguments,
                indexed_getter,
                named_getter,
                extended_attributes,
            } = member
            else {
                panic!("operation")
            };
            assert!(name.is_empty());
            assert_eq!(return_type, expected_type);
            assert_eq!(
                return_type_descriptor,
                &TypeDescriptor::Named(expected_type.into())
            );
            assert_eq!(required_arity(arguments), arity);
            assert!(!indexed_getter);
            assert_eq!(*named_getter, getter);
            assert_eq!(
                has_extended_attribute(extended_attributes, "CEReactions"),
                reactions
            );
        }
        let generated = emit_binding_registry(&definitions).unwrap();
        assert_eq!(generated.matches("name: \"\"").count(), 3);
        assert!(!generated.contains("name: \"undefined\""));
        assert!(!generated.contains("Some(\"setter\")"));
        assert!(!generated.contains("Some(\"deleter\")"));
        let indexed = parse_interface("interface Values { [CEReactions] setter undefined (unsigned long index, Element? value); };").unwrap();
        assert!(matches!(&indexed.members[0], Member::Operation {
            name, return_type, arguments, indexed_getter: false, named_getter: false, ..
        } if name.is_empty() && return_type == "undefined" && required_arity(arguments) == 2));
    }

    #[test]
    fn named_special_operations_and_ordinary_keyword_names_are_preserved() {
        let interface = parse_interface(
            r#"interface Values {
            getter DOMString item(unsigned long index);
            getter DOMString namedItem(DOMString name);
            [CEReactions] setter undefined setItem(DOMString name, DOMString value);
            [CEReactions] deleter boolean removeItem(DOMString name);
            undefined getter(DOMString name);
        };"#,
        )
        .unwrap();
        assert_eq!(interface.members.len(), 5);
        for (member, (expected_name, expected_type, arity, indexed, named, reactions)) in
            interface.members.iter().zip([
                ("item", "DOMString", 1, true, false, false),
                ("namedItem", "DOMString", 1, false, true, false),
                ("setItem", "undefined", 2, false, false, true),
                ("removeItem", "boolean", 1, false, false, true),
                ("getter", "undefined", 1, false, false, false),
            ])
        {
            let Member::Operation {
                name,
                return_type,
                arguments,
                indexed_getter,
                named_getter,
                extended_attributes,
                ..
            } = member
            else {
                panic!("operation")
            };
            assert_eq!(name, expected_name);
            assert_eq!(return_type, expected_type);
            assert_eq!(required_arity(arguments), arity);
            assert_eq!((*indexed_getter, *named_getter), (indexed, named));
            assert_eq!(
                has_extended_attribute(extended_attributes, "CEReactions"),
                reactions
            );
        }
        let generated = emit_binding_registry(&[Definition::Interface(interface)]).unwrap();
        for name in ["item", "namedItem", "setItem", "removeItem", "getter"] {
            assert!(generated.contains(&format!("name: {name:?}")));
        }
    }

    #[test]
    fn anonymous_getter_heads_accept_complete_compound_types() {
        for return_type in [
            "unsigned long",
            "unsigned long?",
            "(Element or DOMString)?",
            "sequence<unsigned long>",
            "record<DOMString, sequence<Element?>>",
        ] {
            let interface = parse_interface(&format!(
                "interface Values {{ getter\t{return_type} (DOMString name); }};"
            ))
            .unwrap();
            let Member::Operation {
                name,
                return_type: actual,
                named_getter,
                ..
            } = &interface.members[0]
            else {
                panic!("operation")
            };
            assert!(name.is_empty());
            assert_eq!(actual, return_type);
            assert!(*named_getter);
        }
    }

    #[test]
    fn special_operations_reject_invalid_keys_arity_and_optional_values() {
        for member in [
            "getter DOMString ()",
            "getter DOMString (DOMString... names)",
            "setter undefined (DOMString name)",
            "setter undefined (boolean key, DOMString value)",
            "setter undefined (DOMString name, optional DOMString value)",
            "setter undefined (DOMString name, DOMString... values)",
            "setter DOMString (DOMString name, DOMString value)",
            "deleter undefined ()",
            "deleter undefined (unsigned long index)",
            "deleter undefined (optional DOMString name)",
            "deleter undefined (DOMString name, DOMString extra)",
        ] {
            assert!(
                parse_interface(&format!("interface Bad {{ {member}; }};")).is_err(),
                "{member}"
            );
        }
    }

    #[test]
    fn named_getters_are_distinct_from_indexed_getters() {
        let definitions = parse_definitions("[Exposed=Window, LegacyUnenumerableNamedProperties] interface HTMLCollection { getter Element? item(unsigned long index); getter Element? namedItem(DOMString name); };").unwrap();
        let interfaces = merge_partial_interfaces(&definitions).unwrap();
        assert!(matches!(
            interfaces[0].members[0],
            Member::Operation {
                indexed_getter: true,
                named_getter: false,
                ..
            }
        ));
        assert!(matches!(
            interfaces[0].members[1],
            Member::Operation {
                indexed_getter: false,
                named_getter: true,
                ..
            }
        ));
        let output = emit_binding_registry(&definitions).unwrap();
        assert!(output.contains("named_getter: true"));
        assert!(parse_interface("interface Bad { getter Element? bad(boolean key); };").is_err());
    }

    #[test]
    fn parses_initial_interface_subset() {
        let interface = parse_interface(
            r#"
                interface Node : EventTarget {
                    readonly attribute DOMString nodeName;
                    attribute DOMString textContent;
                    Node appendChild(Node child);
                };
            "#,
        )
        .expect("parse Node");

        assert_eq!(interface.name, "Node");
        assert_eq!(interface.inherits.as_deref(), Some("EventTarget"));
        assert_eq!(interface.members.len(), 3);
        assert!(emit_rust_metadata(&interface).contains("appendChild"));
    }

    #[test]
    fn indexed_getter_and_value_iteration_metadata_are_preserved() {
        let interface = parse_interface("interface Nodes { getter Node? item(unsigned long index); readonly attribute unsigned long length; iterable<Node>; };").unwrap();
        assert!(matches!(
            &interface.members[0],
            Member::Operation {
                indexed_getter: true,
                ..
            }
        ));
        let output = emit_binding_registry(&[Definition::Interface(interface)]).unwrap();
        assert!(output.contains("indexed_getter: true"));
        assert!(output.contains("value_iterable: true"));
        assert!(
            parse_interface(
                "interface Bad { getter Node? item(unsigned long index, DOMString name); };"
            )
            .is_err()
        );
        assert!(
            parse_interface("interface Bad { getter Node? item(optional unsigned long index); };")
                .is_err()
        );
    }

    #[test]
    fn rejects_unsupported_constructs() {
        let error = parse_interface("interface Node { maplike<DOMString, Node>; };")
            .expect_err("maplike is outside the supported interface set");
        assert!(matches!(error, ParseError::UnsupportedMember(_)));
    }

    #[test]
    fn parses_and_merges_extended_binding_constructs() {
        let definitions = parse_definitions(
            r#"
              interface Response : Body {
                Promise<ArrayBuffer> arrayBuffer();
              };
              partial interface Response {
                Promise<sequence<DOMString?>> values(optional DOMString? prefix = null);
              };
              dictionary RequestInit : BaseInit {
                DOMString method = "GET";
                sequence<DOMString> protocols;
              };
              callback StreamCallback = Promise<void> (optional ArrayBuffer? chunk = null);
            "#,
        )
        .expect("parse extended WebIDL");
        let interfaces = merge_partial_interfaces(&definitions).expect("merge partial interface");
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].members.len(), 2);
        let dictionary = definitions
            .iter()
            .find_map(|definition| match definition {
                Definition::Dictionary(dictionary) => Some(dictionary),
                _ => None,
            })
            .expect("dictionary");
        assert_eq!(dictionary.inherits.as_deref(), Some("BaseInit"));
        assert_eq!(dictionary.members[0].default.as_deref(), Some("\"GET\""));
        let callback = definitions
            .iter()
            .find_map(|definition| match definition {
                Definition::Callback(callback) => Some(callback),
                _ => None,
            })
            .expect("callback");
        assert!(callback.arguments[0].optional);
        assert!(matches!(
            callback.return_type_descriptor,
            TypeDescriptor::Promise(_)
        ));
    }

    #[test]
    fn preserves_executable_extended_attribute_metadata() {
        let interfaces = parse_definitions(
            "[Exposed=(Window,Worker), Constructor(DOMString value)] interface EventTarget { [SameObject] readonly attribute DOMString identity; [CEReactions, PutForwards=value] readonly attribute DOMString forwarded; undefined addEventListener(DOMString type, optional boolean capture = false); };",
        )
        .expect("extended attributes");
        let Definition::Interface(interface) = &interfaces[0] else {
            panic!("interface definition");
        };
        assert_eq!(
            exposure_set(&interface.extended_attributes),
            vec!["Window", "Worker"]
        );
        assert!(has_extended_attribute(
            &interface.extended_attributes,
            "Constructor"
        ));
        let generated = emit_binding_registry(&interfaces).expect("generate registry");
        assert!(generated.contains("EVENT_TARGET_MEMBERS"));
        assert!(generated.contains("addEventListener"));
        assert!(generated.contains("exposed: &[\"Window\", \"Worker\"]"));
        assert!(generated.contains("same_object: true"));
        assert!(generated.contains("ce_reactions: true"));
        assert!(generated.contains("put_forwards: Some(\"value\")"));
        assert!(generated.contains("arity: 1"));
        assert!(generated.contains("constructible: true"));
    }

    #[test]
    fn parses_unions_records_enums_mixins_and_iterables() {
        let definitions = parse_definitions(
            r#"
            interface mixin ParentNode { iterable<Element>; };
            interface Element { constructor(optional (DOMString or long)? value = null); };
            Element includes ParentNode;
            dictionary Bag { record<DOMString, DOMString> values; };
            enum Mode { "open", "closed" };
            "#,
        )
        .expect("expanded WebIDL surface");
        assert!(
            definitions
                .iter()
                .any(|definition| matches!(definition, Definition::InterfaceMixin(_)))
        );
        assert!(
            definitions
                .iter()
                .any(|definition| matches!(definition, Definition::Includes { .. }))
        );
        assert!(
            definitions
                .iter()
                .any(|definition| matches!(definition, Definition::Enumeration(_)))
        );
    }
}

#[cfg(test)]
mod variadic_tests {
    use super::*;
    #[test]
    fn variadic_arguments_have_zero_required_arity_and_preserve_element_type() {
        let args = parse_arguments("DOMString... tokens").unwrap();
        assert!(args[0].variadic);
        assert_eq!(args[0].type_name, "DOMString");
        assert_eq!(required_arity(&args), 0);
        assert_eq!(
            required_arity(&parse_arguments("long first, DOMString... rest").unwrap()),
            1
        );
        for invalid in [
            "optional DOMString... tokens",
            "DOMString... tokens = null",
            "DOMString... tokens, long after",
        ] {
            assert!(parse_arguments(invalid).is_err());
        }
    }
}

#[cfg(test)]
mod reflection_metadata_tests {
    use super::*;

    #[test]
    fn global_handlers_are_shared_without_duplicate_owner_members() {
        let definitions = parse_definitions(include_str!("../idl/browser.webidl")).unwrap();
        let interfaces = merge_partial_interfaces(&definitions).unwrap();
        let Definition::InterfaceMixin(mixin) = definitions.iter().find(|definition|
            matches!(definition, Definition::InterfaceMixin(interface) if interface.name == "GlobalEventHandlers")
        ).unwrap() else { unreachable!() };
        for owner in ["HTMLElement", "Document", "SVGElement"] {
            let interface = interfaces.iter().find(|interface| interface.name == owner).unwrap();
            for member in &mixin.members {
                let Member::Attribute { name, .. } = member else { panic!("handler must be an attribute") };
                assert_eq!(interface.members.iter().filter(|member|
                    matches!(member, Member::Attribute { name: candidate, .. } if candidate == name)
                ).count(), 1, "{owner}.{name}");
            }
        }
    }

    #[test]
    fn global_input_enumerations_are_owned_by_html_not_svg() {
        let definitions = parse_definitions(include_str!("../idl/browser.webidl")).unwrap();
        for (interface_name, expected) in [("HTMLElement", true), ("SVGElement", false)] {
            let interface = definitions.iter().find_map(|definition| match definition {
                Definition::Interface(interface) if interface.name == interface_name => Some(interface),
                _ => None,
            }).unwrap();
            for property in ["inputMode", "enterKeyHint"] {
                let present = interface.members.iter().any(|member| {
                    matches!(member, Member::Attribute { name, .. } if name == property)
                });
                assert_eq!(present, expected, "{interface_name}.{property}");
            }
        }
    }

    #[test]
    fn reflection_names_aliases_and_legacy_conversion_survive_generation() {
        let definitions = parse_definitions(
            r#"[Exposed=Window] interface HTMLExampleElement {
                [CEReactions, Reflect] attribute DOMString bgColor;
                [CEReactions, Reflect=charoff, LegacyNullToEmptyString] attribute DOMString chOff;
                attribute DOMString ordinary;
                [LegacyLenientThis] attribute EventHandler onmouseenter;
            };"#,
        ).unwrap();
        let generated = emit_binding_registry(&definitions).unwrap();
        assert!(generated.contains("reflect_attribute: Some(\"bgcolor\")"));
        assert!(generated.contains("reflect_attribute: Some(\"charoff\")"));
        assert!(generated.contains("legacy_null_to_empty_string: true"));
        assert!(generated.contains("reflect_attribute: None"));
        assert!(generated.contains("ce_reactions: true"));
        assert!(generated.contains("legacy_lenient_this: true"));
        assert!(generated.contains("legacy_lenient_this: false"));
    }
}
