use regex::Regex;

pub fn normalize(input: &str) -> String {
    let mut normalized_lines = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('#') {
            normalized_lines.push(trimmed.to_string());
            continue;
        }

        let (command_part, comment_part) = split_comment(trimmed);

        if command_part.trim().is_empty() {
            if let Some(c) = comment_part {
                normalized_lines.push(format!("#{}", c));
            }
            continue;
        }

        let normalized_command = normalize_command_part(command_part);

        match comment_part {
            Some(comment) => normalized_lines.push(format!("{} #{}", normalized_command, comment)),
            None => normalized_lines.push(normalized_command),
        }
    }

    normalized_lines.join("\n")
}

fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut in_quote = None;
    let mut escaped = false;

    for (idx, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if c == '\\' {
            escaped = true;
            continue;
        }

        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
        } else if c == '"' || c == '\'' {
            in_quote = Some(c);
        } else if c == '#' {
            if idx > 0 {
                let prev_char = line[..idx].chars().last().unwrap();
                if prev_char.is_whitespace() {
                    return (&line[..idx], Some(&line[idx + 1..]));
                }
            } else {
                return ("", Some(&line[1..]));
            }
        }
    }

    (line, None)
}

fn normalize_command_part(input: &str) -> String {
    let tokens = tokenize_and_normalize(input);

    if tokens.is_empty() {
        return input.to_string();
    }

    let mut iter = tokens.into_iter();
    let first = iter.next().unwrap();
    let lower_first = first.to_lowercase();

    let (verb, args) = match lower_first.as_str() {
        "navigate" | "nav" => ("goto".to_string(), iter.collect::<Vec<_>>()),
        "go" => {
            let mut args: Vec<String> = iter.collect();
            if !args.is_empty() && args[0].to_lowercase() == "to" {
                args.remove(0);
                ("goto".to_string(), args)
            } else {
                ("go".to_string(), args)
            }
        }
        "scan" => ("observe".to_string(), iter.collect()),
        "quit" => ("exit".to_string(), iter.collect()),
        "accept" => {
            let mut args: Vec<String> = iter.collect();
            if !args.is_empty() && args[0].to_lowercase() == "cookies" {
                args.remove(0);
                ("accept_cookies".to_string(), args)
            } else {
                ("accept".to_string(), args)
            }
        }
        _ => (lower_first, iter.collect()),
    };

    let mut normalized_args = Vec::new();
    let mut arg_iter = args.into_iter().peekable();

    while let Some(arg) = arg_iter.next() {
        if arg.starts_with("css(") || arg.starts_with("xpath(") {
            let mut selector_expr = arg.clone();
            let mut balance = count_paren_balance(&selector_expr);

            while balance > 0 {
                if let Some(next_arg) = arg_iter.next() {
                    selector_expr.push(' ');
                    selector_expr.push_str(&next_arg);
                    balance += count_paren_balance(&next_arg);
                } else {
                    break;
                }
            }

            if let (Some(open_idx), Some(close_idx)) =
                (selector_expr.find('('), selector_expr.rfind(')'))
            {
                let kind = selector_expr[..open_idx].trim().to_lowercase();
                let inner = selector_expr[open_idx + 1..close_idx].trim();
                if kind == "css" || kind == "xpath" {
                    let normalized_inner = if inner.starts_with('"') && inner.ends_with('"') {
                        inner.to_string()
                    } else if inner.starts_with('\'') && inner.ends_with('\'') {
                        let unquoted = &inner[1..inner.len() - 1];
                        let escaped = unquoted.replace('\\', "\\\\").replace('"', "\\\"");
                        format!("\"{}\"", escaped)
                    } else {
                        let escaped = inner.replace('\\', "\\\\").replace('"', "\\\"");
                        format!("\"{}\"", escaped)
                    };
                    normalized_args.push(format!("{}({})", kind, normalized_inner));
                    continue;
                }
            }
        }

        // Handle unquoted JSON - rejoin split tokens until braces balance
        if arg.starts_with('{') && !arg.starts_with('"') && !arg.starts_with('\'') {
            let mut json_str = arg.clone();
            let mut balance = count_balance(&json_str);

            while balance > 0 {
                match arg_iter.next() {
                    Some(next_arg) => {
                        json_str.push(' ');
                        json_str.push_str(&next_arg);
                        balance += count_balance(&next_arg);
                    }
                    None => break,
                }
            }
            let escaped = json_str.replace('"', "\\\"");
            normalized_args.push(format!("\"{}\"", escaped));
            continue;
        }

        // Option Normalization
        if arg.starts_with("--") {
            normalized_args.push(arg.to_lowercase());
        } else if arg.starts_with("-") && !is_number(&arg) {
            normalized_args.push(format!("-{}", arg.to_lowercase())); // -opt -> --opt
        } else {
            // Context sensitive handling
            let norm = match verb.as_str() {
                "observe" => {
                    if arg.eq_ignore_ascii_case("full") {
                        "--full".to_string()
                    } else if arg.eq_ignore_ascii_case("minimal") {
                        "--minimal".to_string()
                    } else {
                        arg
                    }
                }
                "press" => {
                    let lower = arg.to_lowercase();
                    if arg.contains('+')
                        || matches!(
                            lower.as_str(),
                            "control"
                                | "shift"
                                | "alt"
                                | "meta"
                                | "enter"
                                | "tab"
                                | "escape"
                                | "space"
                                | "backspace"
                                | "delete"
                                | "arrowup"
                                | "arrowdown"
                                | "arrowleft"
                                | "arrowright"
                                | "home"
                                | "end"
                                | "pageup"
                                | "pagedown"
                                | "f1"
                                | "f2"
                                | "f3"
                                | "f4"
                                | "f5"
                                | "f6"
                                | "f7"
                                | "f8"
                                | "f9"
                                | "f10"
                                | "f11"
                                | "f12"
                        )
                    {
                        lower
                    } else {
                        arg
                    }
                }
                "search" => arg, // revert enter -> --enter
                "device" => {
                    if normalized_args.is_empty() && !arg.starts_with('"') {
                        if arg.eq_ignore_ascii_case("reset") {
                            arg
                        } else {
                            let mut dev_name = arg.clone();
                            while let Some(peek) = arg_iter.peek() {
                                if peek.starts_with("-") {
                                    break;
                                }
                                let next = arg_iter.next().unwrap();
                                dev_name.push(' ');
                                dev_name.push_str(&next);
                            }
                            format!("\"{}\"", dev_name)
                        }
                    } else {
                        arg
                    }
                }
                "wait" => arg, // don't touch options

                // Relational keywords - auto-quote following bare words
                _ if !normalized_args.is_empty()
                    && matches!(
                        normalized_args.last().unwrap().to_lowercase().as_str(),
                        "inside" | "near" | "after" | "before" | "contains"
                    ) =>
                {
                    // Auto-quote if not already quoted/id/selector
                    if !arg.starts_with('"')
                        && !arg.starts_with('\'')
                        && !arg.starts_with('#')
                        && !arg.starts_with('@')
                        && !arg.starts_with("css(")
                        && !arg.starts_with("xpath(")
                        && !arg.starts_with('-')
                    {
                        format!("\"{}\"", arg)
                    } else {
                        arg
                    }
                }

                // Commands that expect text targets - auto-quote bare words
                "click" | "hover" | "focus" | "check" | "uncheck" | "select" => {
                    // If this is the first arg and it's not quoted, not an ID, not a selector
                    if normalized_args.is_empty()
                        && !arg.starts_with('"')
                        && !arg.starts_with('\'')
                        && !arg.starts_with('#')
                        && !arg.starts_with('@')
                        && !arg.starts_with("css(")
                        && !arg.starts_with("xpath(")
                        && !arg.starts_with('-')
                    {
                        // Collect all following bare words until we hit an option or relational keyword
                        let mut text = arg.clone();
                        while let Some(peek) = arg_iter.peek() {
                            if peek.starts_with('-')
                                || matches!(
                                    peek.to_lowercase().as_str(),
                                    "inside" | "near" | "after" | "before" | "contains"
                                )
                            {
                                break;
                            }
                            let next = arg_iter.next().unwrap();
                            text.push(' ');
                            text.push_str(&next);
                        }
                        format!("\"{}\"", text)
                    } else {
                        arg
                    }
                }

                // Type command - auto-quote first two bare word sequences separately
                "type" => {
                    // Check if this should be auto-quoted
                    if !arg.starts_with('"')
                        && !arg.starts_with('\'')
                        && !arg.starts_with('#')
                        && !arg.starts_with('@')
                        && !arg.starts_with("css(")
                        && !arg.starts_with("xpath(")
                        && !arg.starts_with('-')
                    {
                        format!("\"{}\"", arg)
                    } else {
                        arg
                    }
                }

                _ => arg,
            };
            normalized_args.push(norm);
        }
    }

    // Post-process specific commands
    if verb == "press" {
        // Consolidate key combo: ["control", "+", "shift"] -> ["control+shift"]
        let mut new_args = Vec::new();
        let mut buffer = String::new();

        for arg in normalized_args {
            if arg == "+" {
                buffer.push('+');
            } else if buffer.ends_with('+') {
                buffer.push_str(&arg.to_lowercase());
            } else {
                if !buffer.is_empty() {
                    new_args.push(buffer);
                }
                buffer = arg.to_lowercase();
            }
        }
        if !buffer.is_empty() {
            new_args.push(buffer);
        }
        normalized_args = new_args;
    } else if verb == "type" {
        // Reorder: move text string after relational parts if present
        let string_idx = normalized_args.iter().position(|arg| arg.starts_with('"'));

        if let Some(idx) = string_idx {
            let has_relational_after = normalized_args.iter().skip(idx + 1).any(|arg| {
                matches!(
                    arg.to_lowercase().as_str(),
                    "inside" | "near" | "after" | "before" | "contains"
                )
            });

            if has_relational_after {
                let text_arg = normalized_args.remove(idx);
                let insert_pos = normalized_args
                    .iter()
                    .position(|arg| arg.starts_with('-'))
                    .unwrap_or(normalized_args.len());
                normalized_args.insert(insert_pos, text_arg);
            }
        }
    }

    // Quote unquoted cookie values
    if verb == "cookies" && normalized_args.len() >= 3 && normalized_args[0] == "set" {
        let val = &normalized_args[2];
        if !val.starts_with('"') && !val.starts_with('\'') {
            normalized_args[2] = format!("\"{}\"", val);
        }
    }

    std::iter::once(verb)
        .chain(normalized_args)
        .collect::<Vec<_>>()
        .join(" ")
}

fn count_balance(s: &str) -> i32 {
    s.chars().fold(0, |acc, c| match c {
        '{' => acc + 1,
        '}' => acc - 1,
        _ => acc,
    })
}

fn count_paren_balance(s: &str) -> i32 {
    let mut b = 0;
    let mut in_quote = None;
    let mut escaped = false;

    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
            continue;
        }
        if c == '"' || c == '\'' {
            in_quote = Some(c);
            continue;
        }
        if c == '(' {
            b += 1;
        } else if c == ')' {
            b -= 1;
        }
    }
    b
}

fn tokenize_and_normalize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_token = String::new();

    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            if !current_token.is_empty() {
                tokens.push(current_token);
                current_token = String::new();
            }
            continue;
        }

        if c == '"' {
            current_token.push('"');
            while let Some(ic) = chars.next() {
                if ic == '\\' {
                    current_token.push('\\');
                    if let Some(next_c) = chars.next() {
                        current_token.push(next_c);
                    }
                } else if ic == '"' {
                    current_token.push('"');
                    break;
                } else {
                    current_token.push(ic);
                }
            }
        } else if c == '\'' {
            current_token.push('"');
            while let Some(ic) = chars.next() {
                if ic == '\\' {
                    if let Some(next_c) = chars.next() {
                        if next_c == '\'' {
                            current_token.push('\'');
                        } else {
                            current_token.push('\\');
                            current_token.push(next_c);
                        }
                    } else {
                        current_token.push('\\');
                    }
                } else if ic == '\'' {
                    current_token.push('"');
                    break;
                } else if ic == '"' {
                    current_token.push('\\');
                    current_token.push('"');
                } else {
                    current_token.push(ic);
                }
            }
        } else {
            current_token.push(c);
        }
    }

    if !current_token.is_empty() {
        tokens.push(current_token);
    }

    tokens
}

fn is_number(s: &str) -> bool {
    use std::sync::LazyLock;
    static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?\d+(\.\d+)?$").unwrap());
    NUMBER_RE.is_match(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_quote_click() {
        assert_eq!(normalize("click store"), "click \"store\"");
        assert_eq!(normalize("click Add to Cart"), "click \"Add to Cart\"");
        assert_eq!(
            normalize("click Submit --force"),
            "click \"Submit\" --force"
        );
    }

    #[test]
    fn test_auto_quote_type() {
        assert_eq!(
            normalize("type email test@example.com"),
            "type \"email\" \"test@example.com\""
        );
    }

    #[test]
    fn test_dont_quote_ids() {
        assert_eq!(normalize("click #5"), "click #5");
        assert_eq!(normalize("click @button"), "click @button");
    }

    #[test]
    fn test_dont_quote_already_quoted() {
        assert_eq!(normalize("click \"Submit\""), "click \"Submit\"");
        assert_eq!(normalize("click 'Submit'"), "click \"Submit\""); // single -> double
    }

    #[test]
    fn test_dont_quote_selectors() {
        assert_eq!(normalize("click css(.button)"), "click css(\".button\")");
        assert_eq!(
            normalize("click xpath(//button)"),
            "click xpath(\"//button\")"
        );
    }

    #[test]
    fn test_relational_keywords() {
        assert_eq!(
            normalize("click Submit inside form"),
            "click \"Submit\" inside \"form\""
        );
    }
}
