use super::{Error, Result};
#[derive(Clone, Debug)]
pub(crate) enum Expr {
    String(String),
    Number(f64),
    Context,
    Root,
    Function(String, Vec<Expr>),
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Filter(Box<Expr>, Vec<Expr>),
    Path(Box<Expr>, Vec<Step>),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Union,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Axis {
    Child,
    Descendant,
    Parent,
    Ancestor,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    Attribute,
    Namespace,
    SelfNode,
    DescendantOrSelf,
    AncestorOrSelf,
}
#[derive(Clone, Debug)]
pub(crate) enum Test {
    Any,
    Node,
    Text,
    Comment,
    Pi(Option<String>),
    Name {
        namespace: String,
        local: Option<String>,
        unprefixed: bool,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub axis: Axis,
    pub test: Test,
    pub predicates: Vec<Expr>,
}
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    String(String),
    Number(f64),
    Slash,
    DoubleSlash,
    Dot,
    DoubleDot,
    At,
    Axis,
    Left,
    Right,
    Open,
    Close,
    Comma,
    Plus,
    Minus,
    Star,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Pipe,
    End,
}
fn name_start(c: char) -> bool {
    c == '_'
        || c.is_ascii_alphabetic()
        || matches!(c as u32,0xc0..=0xd6|0xd8..=0xf6|0xf8..=0x2ff|0x370..=0x37d|0x37f..=0x1fff|0x200c..=0x200d|0x2070..=0x218f|0x2c00..=0x2fef|0x3001..=0xd7ff|0xf900..=0xfdcf|0xfdf0..=0xfffd|0x10000..=0xeffff)
}
fn name_char(c: char) -> bool {
    name_start(c)
        || c.is_ascii_digit()
        || matches!(c, '-' | '.' | '\u{b7}')
        || matches!(c as u32,0x300..=0x36f|0x203f..=0x2040)
}
fn tokenize(source: &str) -> Result<Vec<Token>> {
    if source.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    let units = crate::dom_string::units(source);
    let mut chars = Vec::new();
    let mut offsets = Vec::new();
    let mut unpaired = Vec::new();
    let mut index = 0;
    while index < units.len() {
        offsets.push(index);
        let first = units[index];
        if (0xd800..=0xdbff).contains(&first)
            && units
                .get(index + 1)
                .is_some_and(|u| (0xdc00..=0xdfff).contains(u))
        {
            let value =
                0x10000 + (((first as u32 - 0xd800) << 10) | (units[index + 1] as u32 - 0xdc00));
            chars.push(char::from_u32(value).unwrap());
            unpaired.push(false);
            index += 2;
        } else {
            chars.push(char::from_u32(first as u32).unwrap_or('\u{fffd}'));
            unpaired.push((0xd800..=0xdfff).contains(&first));
            index += 1;
        }
    }
    offsets.push(units.len());
    let mut at = 0;
    let mut tokens = Vec::new();
    while at < chars.len() {
        if unpaired[at] {
            return Err(Error::Syntax("unpaired surrogate outside literal".into()));
        }
        let c = chars[at];
        if matches!(c, ' ' | '\t' | '\r' | '\n') {
            at += 1;
            continue;
        }
        let next = chars.get(at + 1).copied();
        let token = match (c, next) {
            ('/', Some('/')) => {
                at += 2;
                Token::DoubleSlash
            }
            (':', Some(':')) => {
                at += 2;
                Token::Axis
            }
            ('.', Some('.')) => {
                at += 2;
                Token::DoubleDot
            }
            ('!', Some('=')) => {
                at += 2;
                Token::NotEqual
            }
            ('<', Some('=')) => {
                at += 2;
                Token::LessEqual
            }
            ('>', Some('=')) => {
                at += 2;
                Token::GreaterEqual
            }
            ('\'' | '"', _) => {
                at += 1;
                let start = at;
                while at < chars.len() && chars[at] != c {
                    at += 1;
                }
                if at == chars.len() {
                    return Err(Error::Syntax("unterminated literal".into()));
                }
                let value = crate::dom_string::from_units(&units[offsets[start]..offsets[at]]);
                at += 1;
                Token::String(value)
            }
            ('0'..='9', _) | ('.', Some('0'..='9')) => {
                let start = at;
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
                if chars.get(at) == Some(&'.') {
                    at += 1;
                    while at < chars.len() && chars[at].is_ascii_digit() {
                        at += 1;
                    }
                }
                Token::Number(
                    chars[start..at]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .map_err(|_| Error::Syntax("invalid number".into()))?,
                )
            }
            _ if name_start(c) => {
                let start = at;
                at += 1;
                while at < chars.len() && !unpaired[at] && name_char(chars[at]) {
                    at += 1;
                }
                if chars.get(at) == Some(&':') && chars.get(at + 1) != Some(&':') {
                    at += 1;
                    if chars.get(at) == Some(&'*') {
                        at += 1;
                    } else {
                        if !chars.get(at).copied().is_some_and(name_start) {
                            return Err(Error::Syntax("invalid QName".into()));
                        }
                        at += 1;
                        while at < chars.len() && !unpaired[at] && name_char(chars[at]) {
                            at += 1;
                        }
                    }
                }
                Token::Name(chars[start..at].iter().collect())
            }
            _ => {
                at += 1;
                match c {
                    '/' => Token::Slash,
                    '.' => Token::Dot,
                    '@' => Token::At,
                    '(' => Token::Left,
                    ')' => Token::Right,
                    '[' => Token::Open,
                    ']' => Token::Close,
                    ',' => Token::Comma,
                    '+' => Token::Plus,
                    '-' => Token::Minus,
                    '*' => Token::Star,
                    '=' => Token::Equal,
                    '<' => Token::Less,
                    '>' => Token::Greater,
                    '|' => Token::Pipe,
                    _ => return Err(Error::Syntax(format!("unexpected character {c}"))),
                }
            }
        };
        tokens.push(token);
        if tokens.len() > 65536 {
            return Err(Error::Limit);
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}
// Bound the constructed tree as well as recursive parser calls: long left-
// associative unions and paths otherwise evade the recursion counter and can
// overflow evaluation/destruction stacks before the runtime work limit fires.
fn check_depth(root: &Expr) -> Result<()> {
    let mut pending = vec![(root, 1usize)];
    while let Some((expr, depth)) = pending.pop() {
        if depth > 256 {
            return Err(Error::Limit);
        }
        match expr {
            Expr::Neg(e) => pending.push((e, depth + 1)),
            Expr::Binary(_, a, b) => {
                pending.push((a, depth + 1));
                pending.push((b, depth + 1));
            }
            Expr::Function(_, args) => pending.extend(args.iter().map(|e| (e, depth + 1))),
            Expr::Filter(base, predicates) => {
                pending.push((base, depth + 1));
                pending.extend(predicates.iter().map(|e| (e, depth + 1)));
            }
            Expr::Path(base, steps) => {
                pending.push((base, depth + 1));
                for step in steps {
                    pending.extend(step.predicates.iter().map(|e| (e, depth + 1)));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub(crate) fn parse(
    source: &str,
    resolver: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Expr> {
    let mut p = Parser {
        tokens: tokenize(source)?,
        at: 0,
        depth: 0,
        resolved_bytes: 0,
        resolver,
    };
    let expr = p.expression(0)?;
    if p.peek() != &Token::End {
        return Err(Error::Syntax("trailing expression".into()));
    }
    Ok(expr)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    resolved_bytes: usize,
    resolver: &'a mut dyn FnMut(&str) -> Result<String>,
}
impl Parser<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn take(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        if token != Token::End {
            self.at += 1;
        }
        token
    }
    fn expect(&mut self, token: Token) -> Result<()> {
        if self.take() == token {
            Ok(())
        } else {
            Err(Error::Syntax("unexpected token".into()))
        }
    }
    fn is_step(&self) -> bool {
        match self.peek() {
            Token::At | Token::Dot | Token::DoubleDot | Token::Star => true,
            Token::Name(name) => {
                self.tokens.get(self.at + 1) != Some(&Token::Left)
                    || matches!(
                        name.as_str(),
                        "node" | "text" | "comment" | "processing-instruction"
                    )
            }
            _ => false,
        }
    }
    fn expression(&mut self, min: u8) -> Result<Expr> {
        self.depth += 1;
        if self.depth > 256 {
            return Err(Error::Limit);
        }
        let mut left = if self.peek() == &Token::Minus {
            self.take();
            Expr::Neg(Box::new(self.expression(7)?))
        } else {
            self.primary()?
        };
        loop {
            let (op, p) = match self.peek() {
                Token::Name(s) if s == "or" => (Op::Or, 1),
                Token::Name(s) if s == "and" => (Op::And, 2),
                Token::Equal => (Op::Equal, 3),
                Token::NotEqual => (Op::NotEqual, 3),
                Token::Less => (Op::Less, 4),
                Token::LessEqual => (Op::LessEqual, 4),
                Token::Greater => (Op::Greater, 4),
                Token::GreaterEqual => (Op::GreaterEqual, 4),
                Token::Plus => (Op::Add, 5),
                Token::Minus => (Op::Subtract, 5),
                Token::Star => (Op::Multiply, 6),
                Token::Name(s) if s == "div" => (Op::Divide, 6),
                Token::Name(s) if s == "mod" => (Op::Modulo, 6),
                Token::Pipe => (Op::Union, 7),
                _ => break,
            };
            if p < min {
                break;
            }
            self.take();
            let right = self.expression(p + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
            check_depth(&left)?;
        }
        check_depth(&left)?;
        self.depth -= 1;
        Ok(left)
    }
    fn primary(&mut self) -> Result<Expr> {
        let location = matches!(self.peek(), Token::Slash | Token::DoubleSlash) || self.is_step();
        let mut expr = match self.peek() {
            Token::Slash | Token::DoubleSlash => {
                let descendant = self.take() == Token::DoubleSlash;
                let mut steps = Vec::new();
                if descendant {
                    steps.push(descendant_step());
                }
                if self.is_step() {
                    steps.push(self.step()?);
                } else if descendant {
                    return Err(Error::Syntax("missing step".into()));
                }
                Expr::Path(Box::new(Expr::Root), steps)
            }
            _ if self.is_step() => Expr::Path(Box::new(Expr::Context), vec![self.step()?]),
            Token::String(_) => {
                let Token::String(value) = self.take() else {
                    unreachable!()
                };
                Expr::String(value)
            }
            Token::Number(_) => {
                let Token::Number(value) = self.take() else {
                    unreachable!()
                };
                Expr::Number(value)
            }
            Token::Left => {
                self.take();
                let e = self.expression(0)?;
                self.expect(Token::Right)?;
                e
            }
            Token::Name(_) => {
                let Token::Name(name) = self.take() else {
                    unreachable!()
                };
                self.expect(Token::Left)?;
                let mut args = Vec::new();
                if self.peek() != &Token::Right {
                    loop {
                        args.push(self.expression(0)?);
                        if self.peek() != &Token::Comma {
                            break;
                        }
                        self.take();
                    }
                }
                self.expect(Token::Right)?;
                validate_function(&name, args.len())?;
                Expr::Function(name, args)
            }
            _ => return Err(Error::Syntax("expected expression".into())),
        };
        let predicates = self.predicates()?;
        if location && !predicates.is_empty() {
            return Err(Error::Syntax(
                "predicate after abbreviated/root step".into(),
            ));
        }
        if !predicates.is_empty() {
            expr = Expr::Filter(Box::new(expr), predicates);
        }
        while matches!(self.peek(), Token::Slash | Token::DoubleSlash) {
            let descendant = self.take() == Token::DoubleSlash;
            let mut steps = Vec::new();
            if descendant {
                steps.push(descendant_step());
            }
            steps.push(self.step()?);
            expr = Expr::Path(Box::new(expr), steps);
            check_depth(&expr)?;
        }
        Ok(expr)
    }
    fn predicates(&mut self) -> Result<Vec<Expr>> {
        let mut result = Vec::new();
        while self.peek() == &Token::Open {
            self.take();
            result.push(self.expression(0)?);
            self.expect(Token::Close)?;
        }
        Ok(result)
    }
    fn step(&mut self) -> Result<Step> {
        if matches!(self.peek(), Token::Dot | Token::DoubleDot) {
            let axis = if self.take() == Token::Dot {
                Axis::SelfNode
            } else {
                Axis::Parent
            };
            return Ok(Step {
                axis,
                test: Test::Node,
                predicates: Vec::new(),
            });
        }
        let mut axis = Axis::Child;
        if self.peek() == &Token::At {
            self.take();
            axis = Axis::Attribute;
        } else if self.tokens.get(self.at + 1) == Some(&Token::Axis) {
            let Token::Name(name) = self.take() else {
                return Err(Error::Syntax("missing axis name".into()));
            };
            self.take();
            axis = match name.as_str() {
                "child" => Axis::Child,
                "descendant" => Axis::Descendant,
                "parent" => Axis::Parent,
                "ancestor" => Axis::Ancestor,
                "following-sibling" => Axis::FollowingSibling,
                "preceding-sibling" => Axis::PrecedingSibling,
                "following" => Axis::Following,
                "preceding" => Axis::Preceding,
                "attribute" => Axis::Attribute,
                "namespace" => Axis::Namespace,
                "self" => Axis::SelfNode,
                "descendant-or-self" => Axis::DescendantOrSelf,
                "ancestor-or-self" => Axis::AncestorOrSelf,
                _ => return Err(Error::Syntax("unknown axis".into())),
            };
        }
        let test = match self.take() {
            Token::Star => Test::Any,
            Token::Name(name) => {
                if self.peek() == &Token::Left {
                    self.take();
                    let test = match name.as_str() {
                        "node" => Test::Node,
                        "text" => Test::Text,
                        "comment" => Test::Comment,
                        "processing-instruction" => {
                            let target = if let Token::String(_) = self.peek() {
                                let Token::String(v) = self.take() else {
                                    unreachable!()
                                };
                                Some(v)
                            } else {
                                None
                            };
                            Test::Pi(target)
                        }
                        _ => return Err(Error::Syntax("unknown node test".into())),
                    };
                    self.expect(Token::Right)?;
                    test
                } else if let Some((prefix, local)) = name.split_once(':') {
                    let namespace = (self.resolver)(prefix)?;
                    self.resolved_bytes = self
                        .resolved_bytes
                        .checked_add(namespace.len())
                        .ok_or(Error::Limit)?;
                    if self.resolved_bytes > 32 * 1024 * 1024 {
                        return Err(Error::Limit);
                    }
                    Test::Name {
                        namespace,
                        local: (local != "*").then(|| local.into()),
                        unprefixed: false,
                    }
                } else {
                    Test::Name {
                        namespace: String::new(),
                        local: Some(name),
                        unprefixed: true,
                    }
                }
            }
            _ => return Err(Error::Syntax("missing node test".into())),
        };
        Ok(Step {
            axis,
            test,
            predicates: self.predicates()?,
        })
    }
}
fn descendant_step() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: Test::Node,
        predicates: Vec::new(),
    }
}
fn validate_function(name: &str, n: usize) -> Result<()> {
    let valid = match name {
        "last" | "position" | "true" | "false" => n == 0,
        "count" | "id" | "boolean" | "not" | "lang" | "sum" | "floor" | "ceiling" | "round" => {
            n == 1
        }
        "local-name" | "namespace-uri" | "name" | "string" | "string-length"
        | "normalize-space" | "number" => n <= 1,
        "concat" => n >= 2,
        "starts-with" | "contains" | "substring-before" | "substring-after" => n == 2,
        "substring" => n == 2 || n == 3,
        "translate" => n == 3,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::Syntax(format!(
            "unknown function or invalid arity: {name}"
        )))
    }
}
