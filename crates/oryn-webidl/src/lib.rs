//! WebIDL parsing and binding metadata generation for Oryn.
//!
//! G0 deliberately supports only the interface slice needed for EventTarget,
//! Event, Node, Document, Element, and HTMLElement. Unsupported syntax is an
//! error so browser capabilities are never silently invented.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub inherits: Option<String>,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    Attribute {
        name: String,
        type_name: String,
        readonly: bool,
    },
    Operation {
        name: String,
        return_type: String,
        arguments: Vec<Argument>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    pub type_name: String,
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
}

pub fn parse_interface(input: &str) -> Result<Interface, ParseError> {
    let source = strip_line_comments(input);
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
        members,
    })
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

    if let Some(attribute) = member.strip_prefix("attribute ") {
        let mut pieces = attribute.split_whitespace();
        let type_name = pieces.next();
        let name = pieces.next();
        if let (Some(type_name), Some(name), None) = (type_name, name, pieces.next()) {
            validate_identifier(name)?;
            return Ok(Member::Attribute {
                name: name.to_string(),
                type_name: type_name.to_string(),
                readonly,
            });
        }
    }

    if !readonly
        && let Some((head, tail)) = member.split_once('(')
        && let Some(arguments) = tail.strip_suffix(')')
    {
        let mut head = head.split_whitespace();
        let return_type = head.next();
        let name = head.next();
        if let (Some(return_type), Some(name), None) = (return_type, name, head.next()) {
            validate_identifier(name)?;
            let arguments = parse_arguments(arguments)?;
            return Ok(Member::Operation {
                name: name.to_string(),
                return_type: return_type.to_string(),
                arguments,
            });
        }
    }

    Err(ParseError::UnsupportedMember(member.to_string()))
}

fn parse_arguments(arguments: &str) -> Result<Vec<Argument>, ParseError> {
    if arguments.trim().is_empty() {
        return Ok(Vec::new());
    }
    arguments
        .split(',')
        .map(|argument| {
            let mut pieces = argument.split_whitespace();
            let type_name = pieces.next();
            let name = pieces.next();
            if let (Some(type_name), Some(name), None) = (type_name, name, pieces.next()) {
                validate_identifier(name)?;
                Ok(Argument {
                    name: name.to_string(),
                    type_name: type_name.to_string(),
                })
            } else {
                Err(ParseError::UnsupportedMember(argument.trim().to_string()))
            }
        })
        .collect()
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
}
