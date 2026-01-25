use regex::Regex;

pub fn normalize(input: &str) -> String {
    let mut normalized_lines = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Blank lines mixed vector expects them joined possibly?
            // But my normalizer preserves empty lines if the loop continues.
            // If I skip them, I match vector expectation for "multiple-commands" (maybe).
            // But spec says "Canonical output MAY omit empty lines".
            // Vector "blank-lines-mixed" input: "goto ... \n\nobserve\n\n\nclick 5\n"
            // Expect: "goto ... observe click 5"
            // It seems vector expects single line output.
            // This implies joining lines.
            // But valid OIL is newline separated.
            // If I join lines, I produce invalid OIL.
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

        if let Some(comment) = comment_part {
            normalized_lines.push(format!("{} #{}", normalized_command, comment));
        } else {
            normalized_lines.push(normalized_command);
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

    // 1. Verb Normalization
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

    // 2. Command specific normalization
    // JSON quoting, Option modification

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

        // Check for JSON start
        if arg.starts_with('{') && !arg.starts_with('"') && !arg.starts_with('\'') {
            // It's likely unquoted JSON. We need to slurp tokens until braces balance or end.
            // Wait, `tokenize_and_normalize` splits by space.
            // JSON: `{"a": "b"}` -> `{"a":`, `"b"}`
            // We need to rejoin them.
            let mut json_str = arg.clone();
            let mut balance = count_balance(&json_str);

            while balance > 0 {
                if let Some(next_arg) = arg_iter.next() {
                    json_str.push(' '); // restore space
                    json_str.push_str(&next_arg);
                    balance += count_balance(&next_arg);
                } else {
                    break;
                }
            }
            // Now we have the full JSON string. Quote it and escape inner quotes.
            // Inner quotes in `json_str` are unescaped double quotes (from tokenizer).
            // `{"a":"b"}` -> `"{\"a\":\"b\"}"`
            let escaped = json_str.replace("\"", "\\\"");
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
                    if arg.eq_ignore_ascii_case("control") {
                        "control".to_string()
                    }
                    // keep key names
                    else if arg.eq_ignore_ascii_case("shift") {
                        "shift".to_string()
                    } else if arg.eq_ignore_ascii_case("alt") {
                        "alt".to_string()
                    } else if arg.eq_ignore_ascii_case("meta") {
                        "meta".to_string()
                    } else if arg.contains('+') {
                        // Normalize key combo: Control + Shift -> control+shift
                        arg.to_lowercase()
                    } else {
                        // Check for known edit/nav/func keys that must be lowercase
                        let lower = arg.to_lowercase();
                        match lower.as_str() {
                            "enter" | "tab" | "escape" | "space" | "backspace" | "delete"
                            | "arrowup" | "arrowdown" | "arrowleft" | "arrowright" | "home"
                            | "end" | "pageup" | "pagedown" | "f1" | "f2" | "f3" | "f4" | "f5"
                            | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => lower,
                            _ => arg, // preserve case for char keys? e.g. "A"
                        }
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

                _ => arg,
            };
            normalized_args.push(norm);
        }
    }

    // Post-process specific commands
    if verb == "press" {
        // Fix spaces around +
        // args: ["control", "+", "shift", "+", "a"] -> ["control+shift+a"]
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

        // Handle "enter" -> "--enter" IF it is the only key?
        // Vector press-enter expects "--enter".
        // But grammar fails.
        // We will SKIP normalizing to --enter to pass grammar validity (if user prefers grammar).
        // But test suite expects --enter canonical.
        // I will stick to "enter" (no dash) and let test fail normalization for that specific vector,
        // unless I skip the vector.
    } else if verb == "type" {
        // Fix for role-relational: type email "text" inside "Container" -> type email inside "Container" "text"
        // Pattern: [target_part...] [text_string] [relational_part...]
        // We need to move the text string to the end if relational parts follow it.

        // Find the first quoted string argument.
        let mut string_idx = None;
        for (i, arg) in normalized_args.iter().enumerate() {
            if arg.starts_with('"') {
                string_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = string_idx {
            // Check if there are args after it that look like relational keywords (inside, near, etc)
            // Actually, we just check if there are any args after it.
            // But we must be careful not to move it if those args are options like --append.
            // Relational keywords are bare words: inside, near, in, after, before, contains.

            let has_relational_after = normalized_args.iter().skip(idx + 1).any(|arg| {
                let lower = arg.to_lowercase();
                matches!(
                    lower.as_str(),
                    "inside" | "near" | "after" | "before" | "contains"
                )
            });

            if has_relational_after {
                // Move string_idx element to the end. (But before options?)
                // Spec says text is last argument before options?
                // `type <target> <text> [options]`.
                // Options start with `-`.
                // So we should move it to right before first option, or end if no options.

                let text_arg = normalized_args.remove(idx);

                // Find insert position
                let mut insert_pos = normalized_args.len();
                for (i, arg) in normalized_args.iter().enumerate() {
                    if arg.starts_with('-') {
                        insert_pos = i;
                        break;
                    }
                }

                if insert_pos > normalized_args.len() {
                    normalized_args.push(text_arg);
                } else {
                    normalized_args.insert(insert_pos, text_arg);
                }
            }
        }
    }

    // Cookie Set quoting: cookies set <name> <value>
    if verb == "cookies" && normalized_args.len() >= 3 && normalized_args[0] == "set" {
        // args: [set, name, value, ...]
        // 0=set, 1=name, 2=value.
        // If value is not quoted, quote it.
        let val = &normalized_args[2];
        if !val.starts_with('"') && !val.starts_with('\'') {
            normalized_args[2] = format!("\"{}\"", val);
        }
    }

    // Reconstruct
    let mut result = verb;
    for arg in normalized_args {
        result.push(' ');
        result.push_str(&arg);
    }
    result
}

fn count_balance(s: &str) -> i32 {
    let mut b = 0;
    for c in s.chars() {
        if c == '{' {
            b += 1;
        } else if c == '}' {
            b -= 1;
        }
    }
    b
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
    let re = Regex::new(r"^-?\d+(\.\d+)?$").unwrap();
    re.is_match(s)
}
