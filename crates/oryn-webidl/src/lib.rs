//! WebIDL parsing and binding metadata generation for Oryn.
//!
//! The parser stays intentionally small, but represents the binding constructs
//! used by Oryn's generated browser APIs rather than silently flattening them.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Interface(Interface),
    Dictionary(Dictionary),
    Callback(Callback),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub inherits: Option<String>,
    pub partial: bool,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionary {
    pub name: String,
    pub inherits: Option<String>,
    pub partial: bool,
    pub members: Vec<DictionaryMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryMember {
    pub name: String,
    pub type_name: String,
    pub type_descriptor: TypeDescriptor,
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub name: String,
    pub return_type: String,
    pub return_type_descriptor: TypeDescriptor,
    pub arguments: Vec<Argument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDescriptor {
    Named(String),
    Nullable(Box<TypeDescriptor>),
    Sequence(Box<TypeDescriptor>),
    Promise(Box<TypeDescriptor>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    Attribute {
        name: String,
        type_name: String,
        type_descriptor: TypeDescriptor,
        readonly: bool,
    },
    Operation {
        name: String,
        return_type: String,
        return_type_descriptor: TypeDescriptor,
        arguments: Vec<Argument>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    pub type_name: String,
    pub type_descriptor: TypeDescriptor,
    pub optional: bool,
    pub default: Option<String>,
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

pub fn parse_interface(input: &str) -> Result<Interface, ParseError> {
    let source = strip_line_comments(input);
    let partial = source.trim_start().starts_with("partial interface");
    let declaration = source
        .split_once("interface")
        .map(|(_, rest)| rest.trim())
        .ok_or(ParseError::MissingInterface)?;
    let (header, body_and_tail) = declaration
        .split_once('{')
        .ok_or(ParseError::MissingInterface)?;
    let (body, _) = body_and_tail
        .split_once('}')
        .ok_or(ParseError::UnterminatedBody)?;

    let (name, inherits) = if let Some((name, parent)) = header.split_once(':') {
        (name.trim(), Some(parent.trim().to_string()))
    } else {
        (header.trim(), None)
    };
    validate_identifier(name)?;
    if let Some(parent) = &inherits {
        validate_identifier(parent)?;
    }

    let members = body
        .split(';')
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .map(parse_member)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Interface {
        name: name.to_string(),
        inherits,
        partial,
        members,
    })
}

pub fn parse_definitions(input: &str) -> Result<Vec<Definition>, ParseError> {
    split_definitions(&strip_line_comments(input))
        .into_iter()
        .map(|definition| {
            let trimmed = definition.trim();
            if trimmed.starts_with("interface ") || trimmed.starts_with("partial interface ") {
                parse_interface(trimmed).map(Definition::Interface)
            } else if trimmed.starts_with("dictionary ")
                || trimmed.starts_with("partial dictionary ")
            {
                parse_dictionary(trimmed).map(Definition::Dictionary)
            } else if trimmed.starts_with("callback ") {
                parse_callback(trimmed).map(Definition::Callback)
            } else {
                Err(ParseError::UnsupportedDefinition(trimmed.to_string()))
            }
        })
        .collect()
}

pub fn merge_partial_interfaces(definitions: &[Definition]) -> Result<Vec<Interface>, ParseError> {
    let mut merged = std::collections::BTreeMap::<String, Interface>::new();
    for definition in definitions {
        let Definition::Interface(interface) = definition else {
            continue;
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
            existing.partial = existing.partial && interface.partial;
        } else {
            merged.insert(interface.name.clone(), interface.clone());
        }
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
            Member::Attribute { name, .. } | Member::Operation { name, .. } => name,
        };
        output.push_str(&format!("    {:?},\n", name));
    }
    output.push_str("];\n");
    output
}

fn parse_member(member: &str) -> Result<Member, ParseError> {
    let (readonly, member) = member
        .strip_prefix("readonly ")
        .map_or((false, member), |rest| (true, rest.trim()));

    if let Some(attribute) = member.strip_prefix("attribute ")
        && let Some((type_name, name)) = split_type_and_name(attribute)
    {
        validate_identifier(name)?;
        return Ok(Member::Attribute {
            name: name.to_string(),
            type_name: type_name.to_string(),
            type_descriptor: parse_type(type_name)?,
            readonly,
        });
    }

    if !readonly
        && let Some((head, tail)) = member.split_once('(')
        && let Some(arguments) = tail.strip_suffix(')')
        && let Some((return_type, name)) = split_type_and_name(head)
    {
        validate_identifier(name)?;
        let arguments = parse_arguments(arguments)?;
        return Ok(Member::Operation {
            name: name.to_string(),
            return_type: return_type.to_string(),
            return_type_descriptor: parse_type(return_type)?,
            arguments,
        });
    }

    Err(ParseError::UnsupportedMember(member.to_string()))
}

fn parse_arguments(arguments: &str) -> Result<Vec<Argument>, ParseError> {
    if arguments.trim().is_empty() {
        return Ok(Vec::new());
    }
    split_top_level(arguments, ',')
        .into_iter()
        .map(|argument| {
            let argument = argument.trim();
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
                Ok(Argument {
                    name: name.to_string(),
                    type_name: type_name.to_string(),
                    type_descriptor: parse_type(type_name)?,
                    optional,
                    default,
                })
            } else {
                Err(ParseError::UnsupportedMember(argument.trim().to_string()))
            }
        })
        .collect()
}

fn parse_dictionary(input: &str) -> Result<Dictionary, ParseError> {
    let partial = input.trim_start().starts_with("partial dictionary");
    let declaration = input
        .split_once("dictionary")
        .map(|(_, rest)| rest.trim())
        .ok_or(ParseError::MissingInterface)?;
    let (header, body_and_tail) = declaration
        .split_once('{')
        .ok_or(ParseError::MissingInterface)?;
    let (body, _) = body_and_tail
        .split_once('}')
        .ok_or(ParseError::UnterminatedBody)?;
    let (name, inherits) = if let Some((name, parent)) = header.split_once(':') {
        (name.trim(), Some(parent.trim().to_string()))
    } else {
        (header.trim(), None)
    };
    validate_identifier(name)?;
    if let Some(parent) = &inherits {
        validate_identifier(parent)?;
    }
    let members = body
        .split(';')
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .map(|member| {
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
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Dictionary {
        name: name.to_string(),
        inherits,
        partial,
        members,
    })
}

fn parse_callback(input: &str) -> Result<Callback, ParseError> {
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
    })
}

fn parse_type(input: &str) -> Result<TypeDescriptor, ParseError> {
    let input = input.trim();
    if let Some(inner) = input.strip_suffix('?') {
        return Ok(TypeDescriptor::Nullable(Box::new(parse_type(inner)?)));
    }
    for (prefix, constructor) in [
        (
            "sequence<",
            TypeDescriptor::Sequence as fn(Box<TypeDescriptor>) -> TypeDescriptor,
        ),
        ("Promise<", TypeDescriptor::Promise),
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

fn split_type_and_name(input: &str) -> Option<(&str, &str)> {
    let index = input.rfind(char::is_whitespace)?;
    let type_name = input[..index].trim();
    let name = input[index..].trim();
    (!type_name.is_empty() && !name.is_empty()).then_some((type_name, name))
}

fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut depth = 0_i32;
    let mut quoted = None;
    let mut start = 0;
    let mut values = Vec::new();
    for (index, character) in input.char_indices() {
        if let Some(quote) = quoted {
            if character == quote {
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
    let mut definitions = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, character) in input.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            ';' if depth == 0 => {
                if !input[start..=index].trim().is_empty() {
                    definitions.push(input[start..=index].to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if !input[start..].trim().is_empty() {
        definitions.push(input[start..].to_string());
    }
    definitions
}

fn validate_identifier(value: &str) -> Result<(), ParseError> {
    let valid = value.chars().enumerate().all(|(index, character)| {
        character == '_'
            || character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit())
    });
    if value.is_empty() || !valid {
        return Err(ParseError::InvalidIdentifier(value.to_string()));
    }
    Ok(())
}

fn strip_line_comments(input: &str) -> String {
    input
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_g0_interface_slice() {
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
    fn rejects_unsupported_constructs() {
        let error = parse_interface("interface Node { iterable<Node>; };")
            .expect_err("iterable is outside the G0 slice");
        assert!(matches!(error, ParseError::UnsupportedMember(_)));
    }

    #[test]
    fn parses_and_merges_g3_binding_constructs() {
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
        .expect("parse G3 WebIDL");
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
}
