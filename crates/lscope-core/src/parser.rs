use crate::command::*;
use std::collections::HashMap;
use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Word(String),
    String(String), // Quoted string content
    Flag(String),   // --flag (content without --)
    Number(usize),
    Char(char), // Single characters like ( )
}

pub struct Tokenizer<'a> {
    chars: Peekable<Chars<'a>>,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    fn consume_while<F>(&mut self, predicate: F) -> String
    where
        F: Fn(char) -> bool,
    {
        let mut result = String::new();
        while let Some(&c) = self.chars.peek() {
            if predicate(c) {
                result.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        result
    }

    fn parse_quoted_string(&mut self, quote_char: char) -> Option<String> {
        let mut result = String::new();
        self.chars.next(); // Consume opening quote

        let mut escaped = false;
        for c in self.chars.by_ref() {
            if escaped {
                result.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote_char {
                return Some(result);
            } else {
                result.push(c);
            }
        }
        Some(result)
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
            } else {
                break;
            }
        }

        let c = *self.chars.peek()?;

        if c == '(' || c == ')' {
            self.chars.next();
            return Some(Token::Char(c));
        }

        if c == '"' || c == '\'' {
            let s = self.parse_quoted_string(c)?;
            return Some(Token::String(s));
        }

        if c == '-' {
            self.chars.next();
            if self.chars.peek() == Some(&'-') {
                self.chars.next();
            }
            let flag_name = self.consume_while(|c| !c.is_whitespace() && c != '(' && c != ')');
            if flag_name.is_empty() {
                return Some(Token::Word("-".to_string()));
            }
            return Some(Token::Flag(flag_name));
        }

        // Consume word until whitespace OR parens
        let word = self.consume_while(|c| !c.is_whitespace() && c != '(' && c != ')');

        if let Ok(n) = word.parse::<usize>() {
            Some(Token::Number(n))
        } else {
            Some(Token::Word(word))
        }
    }
}

pub struct Parser<'a> {
    tokenizer: Peekable<Tokenizer<'a>>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            tokenizer: Tokenizer::new(input).peekable(),
        }
    }

    pub fn parse(&mut self) -> Result<Vec<Command>, String> {
        let mut commands = Vec::new();
        while self.tokenizer.peek().is_some() {
            commands.push(self.parse_command()?);
        }
        Ok(commands)
    }

    fn peek_token(&mut self) -> Option<&Token> {
        self.tokenizer.peek()
    }

    fn consume_token(&mut self) -> Option<Token> {
        self.tokenizer.next()
    }

    fn parse_command(&mut self) -> Result<Command, String> {
        let token = self.consume_token().ok_or("Unexpected end of input")?;
        match token {
            Token::Word(cmd) => {
                let lower_cmd = cmd.to_lowercase();
                match lower_cmd.as_str() {
                    "goto" => self.parse_goto(),
                    "go" | "navigate" | "visit" => {
                        if matches!(self.peek_token(), Some(Token::Word(w)) if w.to_lowercase() == "to")
                        {
                            self.consume_token();
                        }
                        self.parse_goto()
                    }
                    "back" => Ok(Command::Back),
                    "forward" => Ok(Command::Forward),
                    "refresh" | "reload" => Ok(Command::Refresh(self.parse_options())),
                    "url" => Ok(Command::Url),
                    "observe" | "scan" => Ok(Command::Observe(self.parse_options())),
                    "html" => Ok(Command::Html(self.parse_options())),
                    "text" => Ok(Command::Text(self.parse_options())),
                    "title" => Ok(Command::Title),
                    "screenshot" | "snap" => Ok(Command::Screenshot(self.parse_options())),
                    "click" => self.parse_click(),
                    "type" | "send" | "fill" => self.parse_type(),
                    "clear" => self.parse_clear(),
                    "press" | "key" => self.parse_press(),
                    "select" | "choose" => self.parse_select(),
                    "check" | "mark" => self.parse_check(true),
                    "uncheck" | "unmark" => self.parse_check(false),
                    "hover" => self.parse_hover(),
                    "focus" => self.parse_focus(),
                    "wait" | "sleep" => self.parse_wait(),
                    "extract" => self.parse_extract(),
                    "cookies" | "cookie" => self.parse_cookies(),
                    "storage" => self.parse_storage(),
                    "tabs" | "tab" => self.parse_tabs(),
                    "submit" => self.parse_submit(),
                    "login" => self.parse_login(),
                    "search" => self.parse_search(),
                    "dismiss" => self.parse_dismiss(),
                    "accept" => self.parse_accept(),
                    "scroll" => self.parse_scroll(),
                    "pdf" => self.parse_pdf(),
                    _ => Err(format!("Unknown command: {}", cmd)),
                }
            }
            _ => Err("Expected command word".to_string()),
        }
    }

    // Consumes everything until closing paren
    fn consume_parenthesized_content(&mut self) -> Result<String, String> {
        // Expect '('
        match self.consume_token() {
            Some(Token::Char('(')) => {}
            _ => return Err("Expected '('".to_string()),
        }

        let mut content = String::new();
        let mut nesting = 0;

        loop {
            let token = self.consume_token().ok_or("Unclosed parentheses")?;
            match token {
                Token::Char(')') => {
                    if nesting == 0 {
                        return Ok(content.trim().to_string());
                    } else {
                        content.push(')');
                        nesting -= 1;
                    }
                }
                Token::Char('(') => {
                    content.push('(');
                    nesting += 1;
                }
                Token::Word(w) => {
                    if !content.is_empty() {
                        content.push(' ');
                    }
                    content.push_str(&w);
                }
                Token::String(s) => {
                    if !content.is_empty() {
                        content.push(' ');
                    }
                    content.push('"');
                    content.push_str(&s);
                    content.push('"');
                } // Restore quotes roughly?
                Token::Number(n) => {
                    if !content.is_empty() {
                        content.push(' ');
                    }
                    content.push_str(&n.to_string());
                }
                Token::Flag(f) => {
                    if !content.is_empty() {
                        content.push(' ');
                    }
                    content.push_str("--");
                    content.push_str(&f);
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::while_let_loop)]
    fn parse_target(&mut self) -> Result<Target, String> {
        let mut target = self.parse_base_target()?;

        loop {
            // Check for modifiers
            if let Some(Token::Word(w)) = self.peek_token() {
                match w.to_lowercase().as_str() {
                    "near" => {
                        self.consume_token();
                        let anchor = self.parse_base_target()?; // Don't allow chaining immediately or do?
                        // "click A near B near C" -> (A near B) near C
                        target = Target::Near {
                            target: Box::new(target),
                            anchor: Box::new(anchor),
                        };
                    }
                    "inside" | "in" => {
                        self.consume_token();
                        let container = self.parse_base_target()?;
                        target = Target::Inside {
                            target: Box::new(target),
                            container: Box::new(container),
                        };
                    }
                    "after" => {
                        self.consume_token();
                        let anchor = self.parse_base_target()?;
                        target = Target::After {
                            target: Box::new(target),
                            anchor: Box::new(anchor),
                        };
                    }
                    "before" => {
                        self.consume_token();
                        let anchor = self.parse_base_target()?;
                        target = Target::Before {
                            target: Box::new(target),
                            anchor: Box::new(anchor),
                        };
                    }
                    "contains" => {
                        self.consume_token();
                        let content = self.parse_base_target()?;
                        target = Target::Contains {
                            target: Box::new(target),
                            content: Box::new(content),
                        };
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }

        Ok(target)
    }

    fn parse_base_target(&mut self) -> Result<Target, String> {
        let token = self.consume_token().ok_or("Expected target")?;
        match token {
            Token::Number(n) => Ok(Target::Id(n)),
            Token::String(s) => Ok(Target::Text(s)),
            Token::Word(w) => {
                let lower = w.to_lowercase();
                if lower == "css" || lower == "xpath" {
                    if let Some(Token::Char('(')) = self.peek_token() {
                        let selector = self.consume_parenthesized_content()?;
                        Ok(Target::Selector(selector))
                    } else {
                        Ok(Target::Text(w))
                    }
                } else if self.is_role(&w) {
                    Ok(Target::Role(w))
                } else {
                    Ok(Target::Text(w))
                }
            }
            _ => Err("Expected target".to_string()),
        }
    }

    fn is_role(&self, w: &str) -> bool {
        let roles = [
            "email", "password", "search", "submit", "username", "phone", "url", "link", "button",
            "input", "checkbox", "radio",
        ];
        roles.contains(&w.to_lowercase().as_str())
    }

    fn parse_options(&mut self) -> HashMap<String, String> {
        let mut options = HashMap::new();
        loop {
            let peek = self.peek_token();
            match peek {
                Some(Token::Flag(_)) => {
                    let flag_token = self.consume_token().unwrap();
                    if let Token::Flag(flag_name) = flag_token {
                        let mut val = "true".to_string();
                        if let Some(next_token) = self.peek_token() {
                            match next_token {
                                Token::Flag(_) => {}
                                Token::Word(_) | Token::String(_) | Token::Number(_) => {
                                    val = self.parse_string_arg().unwrap();
                                }
                                _ => {}
                            }
                        }
                        options.insert(flag_name, val);
                    }
                }
                _ => break,
            }
        }
        options
    }

    fn parse_goto(&mut self) -> Result<Command, String> {
        match self.consume_token() {
            Some(Token::Word(url)) | Some(Token::String(url)) => Ok(Command::GoTo(url)),
            _ => Err("Expected URL".to_string()),
        }
    }

    fn parse_click(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        let options = self.parse_options();
        Ok(Command::Click(target, options))
    }

    fn parse_type(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        let text = self.parse_string_arg()?;
        let options = self.parse_options();
        Ok(Command::Type(target, text, options))
    }

    fn parse_clear(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        Ok(Command::Clear(target))
    }

    fn parse_press(&mut self) -> Result<Command, String> {
        let key = self.parse_string_arg()?;
        let options = self.parse_options();
        Ok(Command::Press(key, options))
    }

    fn parse_select(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        let value = self.parse_string_arg()?;
        Ok(Command::Select(target, value))
    }

    fn parse_check(&mut self, state: bool) -> Result<Command, String> {
        let target = self.parse_target()?;
        if state {
            Ok(Command::Check(target))
        } else {
            Ok(Command::Uncheck(target))
        }
    }

    fn parse_hover(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        Ok(Command::Hover(target))
    }

    fn parse_focus(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        Ok(Command::Focus(target))
    }

    fn parse_wait(&mut self) -> Result<Command, String> {
        let condition_word = match self.consume_token() {
            Some(Token::Word(w)) => w.to_lowercase(),
            _ => return Err("Expected wait condition".to_string()),
        };

        let condition = match condition_word.as_str() {
            "load" => WaitCondition::Load,
            "idle" => WaitCondition::Idle,
            "visible" => WaitCondition::Visible(self.parse_target()?),
            "hidden" => WaitCondition::Hidden(self.parse_target()?),
            "exists" => {
                match self.peek_token() {
                    Some(Token::Word(w)) if w.to_lowercase() == "css" => {
                        self.consume_token(); // consume css
                        let selector = self.consume_parenthesized_content()?;
                        WaitCondition::Exists(selector)
                    }
                    _ => WaitCondition::Exists(self.parse_string_arg()?),
                }
            }
            "gone" => WaitCondition::Gone(self.parse_string_arg()?),
            "url" => WaitCondition::Url(self.parse_string_arg()?),
            _ => return Err(format!("Unknown wait condition: {}", condition_word)),
        };

        let options = self.parse_options();
        Ok(Command::Wait(condition, options))
    }

    fn parse_extract(&mut self) -> Result<Command, String> {
        let source_token = self.consume_token().ok_or("Expected extract source")?;
        let source = match source_token {
            Token::Word(w) => match w.to_lowercase().as_str() {
                "links" => ExtractSource::Links,
                "images" => ExtractSource::Images,
                "tables" => ExtractSource::Tables,
                "meta" => ExtractSource::Meta,
                "css" => {
                    let selector = self.consume_parenthesized_content()?;
                    ExtractSource::Css(selector)
                }
                _ => return Err(format!("Unknown extraction source: {}", w)),
            },
            _ => return Err("Invalid extract source".to_string()),
        };
        Ok(Command::Extract(source))
    }

    fn parse_cookies(&mut self) -> Result<Command, String> {
        let action_word = match self.consume_token() {
            Some(Token::Word(w)) => w.to_lowercase(),
            _ => return Err("Expected cookie action".to_string()),
        };

        let action = match action_word.as_str() {
            "list" => CookieAction::List,
            "get" => CookieAction::Get(self.parse_string_arg()?),
            "delete" => CookieAction::Delete(self.parse_string_arg()?),
            "set" => {
                let name = self.parse_string_arg()?;
                let value = self.parse_string_arg()?;
                CookieAction::Set(name, value)
            }
            _ => return Err("Unknown cookie action".to_string()),
        };
        Ok(Command::Cookies(action))
    }

    fn parse_storage(&mut self) -> Result<Command, String> {
        // syntax: storage "operation" ? ... spec is vague: "Manage localStorage"
        // Command enum says: Storage(String). Maybe just the operation/args as string?
        // Or we should update Command to be more specific?
        // Spec 3.6 says: "storage — Manage localStorage/sessionStorage" with no details.
        // Let's implement it as consuming the rest of the line or a string arg for now?
        // Or "storage clear", "storage get foo".
        // Let's assume it takes an Action-like string.

        let op = self.parse_string_arg()?;
        Ok(Command::Storage(op))
    }

    fn parse_tabs(&mut self) -> Result<Command, String> {
        if let Some(Token::Word(w)) = self.peek_token() {
            let lower = w.to_lowercase();
            match lower.as_str() {
                "new" => {
                    self.consume_token();
                    let url = self.parse_string_arg().unwrap_or_default();
                    return Ok(Command::Tabs(TabAction::New(url)));
                }
                "switch" => {
                    self.consume_token();
                    let id = self.parse_string_arg()?;
                    return Ok(Command::Tabs(TabAction::Switch(id)));
                }
                "close" => {
                    self.consume_token();
                    let id = self.parse_string_arg()?;
                    return Ok(Command::Tabs(TabAction::Close(id)));
                }
                _ => {}
            }
        }
        Ok(Command::Tabs(TabAction::List))
    }

    fn parse_submit(&mut self) -> Result<Command, String> {
        let target = self.parse_target()?;
        Ok(Command::Submit(target))
    }

    fn parse_login(&mut self) -> Result<Command, String> {
        // syntax: login "user" "pass"
        let user = self.parse_string_arg()?;
        let pass = self.parse_string_arg()?;
        let options = self.parse_options();
        Ok(Command::Login(user, pass, options))
    }

    fn parse_search(&mut self) -> Result<Command, String> {
        // syntax: search "query"
        let query = self.parse_string_arg()?;
        let options = self.parse_options();
        Ok(Command::Search(query, options))
    }

    fn parse_dismiss(&mut self) -> Result<Command, String> {
        // syntax: dismiss popups | modals
        let target = match self.consume_token() {
            Some(Token::Word(w)) => w,
            Some(Token::String(s)) => s,
            _ => return Err("Expected what to dismiss".into()),
        };
        let options = self.parse_options();
        Ok(Command::Dismiss(target, options))
    }

    fn parse_accept(&mut self) -> Result<Command, String> {
        // syntax: accept cookies
        let target = match self.consume_token() {
            Some(Token::Word(w)) => w,
            Some(Token::String(s)) => s,
            _ => return Err("Expected what to accept".into()),
        };
        let options = self.parse_options();
        Ok(Command::Accept(target, options))
    }

    #[allow(clippy::collapsible_if)]
    fn parse_scroll(&mut self) -> Result<Command, String> {
        // scroll [target] [direction]
        // or scroll until <target>

        // Handle "scroll until <target>"
        if let Some(Token::Word(w)) = self.peek_token() {
            if w.to_lowercase() == "until" {
                self.consume_token(); // consume 'until'
                let target = self.parse_target()?;
                let options = self.parse_options();
                return Ok(Command::ScrollUntil(target, ScrollDirection::Down, options));
            }
        }

        // Normal scroll
        let mut target = None;
        let mut direction = ScrollDirection::Down;

        // Try to parse target or direction
        // This is tricky because target can be "main" (word) and direction "down" (word)
        // If next is "until", handled above.

        // Peek
        if let Some(token) = self.peek_token() {
            match token {
                Token::Word(w) => {
                    let lower = w.to_lowercase();
                    match lower.as_str() {
                        "up" => {
                            self.consume_token();
                            direction = ScrollDirection::Up;
                        }
                        "down" => {
                            self.consume_token();
                            direction = ScrollDirection::Down;
                        }
                        "left" => {
                            self.consume_token();
                            direction = ScrollDirection::Left;
                        }
                        "right" => {
                            self.consume_token();
                            direction = ScrollDirection::Right;
                        }
                        _ => {
                            // Assume it's a target if not a direction keyword
                            target = Some(self.parse_target()?);

                            // After target, check for direction
                            if let Some(Token::Word(d)) = self.peek_token() {
                                let ld = d.to_lowercase();
                                match ld.as_str() {
                                    "up" => {
                                        self.consume_token();
                                        direction = ScrollDirection::Up;
                                    }
                                    "down" => {
                                        self.consume_token();
                                        direction = ScrollDirection::Down;
                                    }
                                    "left" => {
                                        self.consume_token();
                                        direction = ScrollDirection::Left;
                                    }
                                    "right" => {
                                        self.consume_token();
                                        direction = ScrollDirection::Right;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Token::Number(_) | Token::String(_) => {
                    target = Some(self.parse_target()?);
                    // Check direction
                    if let Some(Token::Word(d)) = self.peek_token() {
                        match d.to_lowercase().as_str() {
                            "up" => {
                                self.consume_token();
                                direction = ScrollDirection::Up;
                            }
                            "down" => {
                                self.consume_token();
                                direction = ScrollDirection::Down;
                            }
                            "left" => {
                                self.consume_token();
                                direction = ScrollDirection::Left;
                            }
                            "right" => {
                                self.consume_token();
                                direction = ScrollDirection::Right;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        let mut options = self.parse_options();

        // Inject parsed direction into options if not present
        if !options.contains_key("direction") {
            let dir_str = match direction {
                ScrollDirection::Up => "up",
                ScrollDirection::Down => "down",
                ScrollDirection::Left => "left",
                ScrollDirection::Right => "right",
            };
            options.insert("direction".to_string(), dir_str.to_string());
        }

        Ok(Command::Scroll(target, options))
    }

    fn parse_pdf(&mut self) -> Result<Command, String> {
        // syntax: pdf "path/to/output.pdf"
        // optional: options?
        let path = self.parse_string_arg()?;
        Ok(Command::Pdf(path))
    }

    fn parse_string_arg(&mut self) -> Result<String, String> {
        match self.consume_token() {
            Some(Token::Word(w)) | Some(Token::String(w)) => Ok(w),
            Some(Token::Number(n)) => Ok(n.to_string()),
            _ => Err("Expected string argument".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(input: &str) -> Command {
        let mut parser = Parser::new(input);
        parser.parse_command().unwrap()
    }

    #[test]
    fn test_tokenizer_basic() {
        let input = "click 5 --force";
        let tokens: Vec<Token> = Tokenizer::new(input).collect();
        assert_eq!(
            tokens,
            vec![
                Token::Word("click".to_string()),
                Token::Number(5),
                Token::Flag("force".to_string())
            ]
        );
    }

    #[test]
    fn test_tokenizer_quotes() {
        let input = "type \"hello world\" 'test string'";
        let tokens: Vec<Token> = Tokenizer::new(input).collect();
        assert_eq!(
            tokens,
            vec![
                Token::Word("type".to_string()),
                Token::String("hello world".to_string()),
                Token::String("test string".to_string())
            ]
        );
    }

    #[test]
    fn test_tokenizer_parens() {
        let input = "css(.btn)";
        let tokens: Vec<Token> = Tokenizer::new(input).collect();
        assert_eq!(
            tokens,
            vec![
                Token::Word("css".to_string()),
                Token::Char('('),
                Token::Word(".btn".to_string()),
                Token::Char(')')
            ]
        );
    }

    #[test]
    fn test_tokenizer_escaping() {
        let input = "type \"hello \\\"world\\\"\"";
        let tokens: Vec<Token> = Tokenizer::new(input).collect();
        assert_eq!(
            tokens,
            vec![
                Token::Word("type".to_string()),
                Token::String("hello \"world\"".to_string())
            ]
        );
    }

    #[test]
    fn test_tokenizer_flags() {
        let input = "refresh -hard --full";
        let tokens: Vec<Token> = Tokenizer::new(input).collect();
        assert_eq!(
            tokens,
            vec![
                Token::Word("refresh".to_string()),
                Token::Flag("hard".to_string()),
                Token::Flag("full".to_string())
            ]
        );
    }

    #[test]
    fn test_click_commands() {
        assert_eq!(
            parse_one("click 5"),
            Command::Click(Target::Id(5), HashMap::new())
        );
        assert_eq!(
            parse_one("click \"Sign In\""),
            Command::Click(Target::Text("Sign In".to_string()), HashMap::new())
        );
        assert_eq!(
            parse_one("click email"),
            Command::Click(Target::Role("email".to_string()), HashMap::new())
        );

        let mut opts = HashMap::new();
        opts.insert("force".to_string(), "true".to_string());
        assert_eq!(
            parse_one("click 5 --force"),
            Command::Click(Target::Id(5), opts)
        );
    }

    #[test]
    fn test_type_commands() {
        assert_eq!(
            parse_one("type 1 \"password123\""),
            Command::Type(Target::Id(1), "password123".to_string(), HashMap::new())
        );
        assert_eq!(
            parse_one("type email \"test@user.com\""),
            Command::Type(
                Target::Role("email".to_string()),
                "test@user.com".to_string(),
                HashMap::new()
            )
        );
    }

    #[test]
    fn test_goto_variations() {
        assert_eq!(
            parse_one("goto google.com"),
            Command::GoTo("google.com".to_string())
        );
        assert_eq!(
            parse_one("go to google.com"),
            Command::GoTo("google.com".to_string())
        );
        assert_eq!(
            parse_one("navigate google.com"),
            Command::GoTo("google.com".to_string())
        );
        assert_eq!(
            parse_one("navigate to google.com"),
            Command::GoTo("google.com".to_string())
        );
    }

    #[test]
    fn test_wait_conditions() {
        assert_eq!(
            parse_one("wait load"),
            Command::Wait(WaitCondition::Load, HashMap::new())
        );
        assert_eq!(
            parse_one("wait visible 5"),
            Command::Wait(WaitCondition::Visible(Target::Id(5)), HashMap::new())
        );
    }

    #[test]
    fn test_selectors() {
        // Now supporting css(...)
        match parse_one("click css(.btn)") {
            Command::Click(Target::Selector(s), _) => assert_eq!(s, ".btn"),
            _ => panic!("Expected selector"),
        }
    }
}
