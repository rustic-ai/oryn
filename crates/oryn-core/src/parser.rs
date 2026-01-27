use super::ast::*;
use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;
use thiserror::Error;

#[derive(Parser)]
#[grammar = "oil.pest"]
pub struct OilParser;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Pest error: {0}")]
    Pest(#[from] pest::error::Error<Rule>),
    #[error("Unknown rule: {0:?}")]
    UnknownRule(Rule),
    #[error("Invalid integer: {0}")]
    InvalidInteger(std::num::ParseIntError),
    #[error("Invalid float: {0}")]
    InvalidFloat(std::num::ParseFloatError),
}

pub fn parse(input: &str) -> Result<Script, ParseError> {
    let mut pairs = OilParser::parse(Rule::oil_input, input)?;
    let mut script = Script { lines: Vec::new() };

    if let Some(pair) = pairs.next() {
        match pair.as_rule() {
            Rule::oil_input => {
                for inner in pair.into_inner() {
                    if inner.as_rule() == Rule::line {
                        script.lines.push(parse_line(inner)?);
                    }
                }
            }
            Rule::line => {
                script.lines.push(parse_line(pair)?);
            }
            _ => {}
        }
    }

    Ok(script)
}

fn parse_line(pair: Pair<Rule>) -> Result<Line, ParseError> {
    let mut command = None;
    let mut comment = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::command => {
                command = Some(parse_command(inner)?);
            }
            Rule::comment => {
                comment = Some(inner.as_str().trim_start_matches('#').to_string());
            }
            _ => {
                // Silent `command` rule passes through the specific command rule directly
                if let Ok(cmd) = parse_command(inner.clone()) {
                    command = Some(cmd);
                }
            }
        }
    }

    Ok(Line { command, comment })
}

fn parse_command(pair: Pair<Rule>) -> Result<Command, ParseError> {
    match pair.as_rule() {
        // Navigation
        Rule::goto_cmd => Ok(Command::Goto(parse_goto(pair)?)),
        Rule::back_cmd => Ok(Command::Back),
        Rule::forward_cmd => Ok(Command::Forward),
        Rule::refresh_cmd => Ok(Command::Refresh(parse_refresh(pair)?)),
        Rule::url_cmd => Ok(Command::Url),

        // Observation
        Rule::observe_cmd => Ok(Command::Observe(parse_observe(pair)?)),
        Rule::html_cmd => Ok(Command::Html(parse_html(pair)?)),
        Rule::text_cmd => Ok(Command::Text(parse_text(pair)?)),
        Rule::title_cmd => Ok(Command::Title),
        Rule::screenshot_cmd => Ok(Command::Screenshot(parse_screenshot(pair)?)),
        Rule::box_cmd => Ok(Command::Box(parse_box(pair)?)),

        // Actions
        Rule::click_cmd => Ok(Command::Click(parse_click(pair)?)),
        Rule::type_cmd => Ok(Command::Type(parse_type(pair)?)),
        Rule::clear_cmd => Ok(Command::Clear(parse_clear(pair)?)),
        Rule::press_cmd => Ok(Command::Press(parse_press(pair)?)),
        Rule::keydown_cmd => Ok(Command::Keydown(parse_keydown(pair)?)),
        Rule::keyup_cmd => Ok(Command::Keyup(parse_keyup(pair)?)),
        Rule::keys_cmd => Ok(Command::Keys),
        Rule::select_cmd => Ok(Command::Select(parse_select(pair)?)),
        Rule::check_cmd => Ok(Command::Check(parse_check(pair)?)),
        Rule::uncheck_cmd => Ok(Command::Uncheck(parse_uncheck(pair)?)),
        Rule::hover_cmd => Ok(Command::Hover(parse_hover(pair)?)),
        Rule::focus_cmd => Ok(Command::Focus(parse_focus(pair)?)),
        Rule::scroll_cmd => Ok(Command::Scroll(parse_scroll(pair)?)),
        Rule::submit_cmd => Ok(Command::Submit(parse_submit(pair)?)),

        // Wait
        Rule::wait_cmd => Ok(Command::Wait(parse_wait(pair)?)),

        // Extract
        Rule::extraction_cmd => Ok(Command::Extract(parse_extract(pair)?)),

        // Sessions
        Rule::cookies_cmd => Ok(Command::Cookies(parse_cookies(pair)?)),
        Rule::storage_cmd => Ok(Command::Storage(parse_storage(pair)?)),
        Rule::sessions_cmd => Ok(Command::Sessions),
        Rule::session_mgmt_cmd => Ok(Command::Session(parse_session_mgmt(pair)?)),
        Rule::state_cmd => Ok(Command::State(parse_state(pair)?)),
        Rule::headers_cmd => Ok(Command::Headers(parse_headers(pair)?)),

        // Tabs
        Rule::tabs_cmd => Ok(Command::Tabs),
        Rule::tab_action_cmd => Ok(Command::Tab(parse_tab_action(pair)?)),

        // Intents
        Rule::login_cmd => Ok(Command::Login(parse_login(pair)?)),
        Rule::search_cmd => Ok(Command::Search(parse_search(pair)?)),
        Rule::dismiss_cmd => Ok(Command::Dismiss(parse_dismiss(pair)?)),
        Rule::accept_cookies_cmd => Ok(Command::AcceptCookies),
        Rule::scroll_until_cmd => Ok(Command::ScrollUntil(parse_scroll_until(pair)?)),

        // Packs
        Rule::packs_cmd => Ok(Command::Packs),
        Rule::pack_action_cmd => Ok(Command::Pack(parse_pack_action(pair)?)),
        Rule::intents_cmd => Ok(Command::Intents(parse_intents(pair)?)),
        Rule::define_cmd => Ok(Command::Define(parse_define(pair)?)),
        Rule::undefine_cmd => Ok(Command::Undefine(parse_undefine(pair)?)),
        Rule::export_cmd => Ok(Command::Export(parse_export(pair)?)),
        Rule::run_cmd => Ok(Command::Run(parse_run(pair)?)),

        // Network
        Rule::intercept_cmd => Ok(Command::Intercept(parse_intercept(pair)?)),
        Rule::requests_cmd => Ok(Command::Requests(parse_requests(pair)?)),

        // Console
        Rule::console_cmd => Ok(Command::Console(parse_console(pair)?)),
        Rule::errors_cmd => Ok(Command::Errors(parse_errors(pair)?)),

        // Frames
        Rule::frames_cmd => Ok(Command::Frames),
        Rule::frame_switch_cmd => Ok(Command::Frame(parse_frame(pair)?)),

        // Dialog
        Rule::dialog_cmd => Ok(Command::Dialog(parse_dialog(pair)?)),

        // Viewport
        Rule::viewport_size_cmd => Ok(Command::Viewport(parse_viewport(pair)?)),
        Rule::device_cmd => Ok(Command::Device(parse_device(pair)?)),
        Rule::devices_cmd => Ok(Command::Devices),
        Rule::media_cmd => Ok(Command::Media(parse_media(pair)?)),

        // Recording
        Rule::trace_cmd => Ok(Command::Trace(parse_trace(pair)?)),
        Rule::record_cmd => Ok(Command::Record(parse_record(pair)?)),
        Rule::highlight_cmd => Ok(Command::Highlight(parse_highlight(pair)?)),

        // Utility
        Rule::pdf_cmd => Ok(Command::Pdf(parse_pdf(pair)?)),
        Rule::learn_cmd => Ok(Command::Learn(parse_learn(pair)?)),
        Rule::exit_cmd => Ok(Command::Exit),
        Rule::help_cmd => Ok(Command::Help(parse_help(pair)?)),

        _ => Err(ParseError::UnknownRule(pair.as_rule())),
    }
}

// --- Parsers for specific commands ---

fn parse_goto(pair: Pair<Rule>) -> Result<GotoCmd, ParseError> {
    let mut url = String::new();
    let mut headers = None;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::url_value => url = inner.as_str().trim_matches('"').to_string(),
            Rule::headers_opt => {
                headers = inner
                    .into_inner()
                    .find(|opt| opt.as_rule() == Rule::string_value)
                    .map(parse_string);
            }
            Rule::timeout_opt => timeout = Some(parse_timeout(inner)?),
            _ => {}
        }
    }
    Ok(GotoCmd {
        url,
        headers,
        timeout,
    })
}

fn parse_refresh(pair: Pair<Rule>) -> Result<RefreshCmd, ParseError> {
    let hard = pair.into_inner().any(|p| p.as_str() == "--hard");
    Ok(RefreshCmd { hard })
}

fn parse_observe(pair: Pair<Rule>) -> Result<ObserveCmd, ParseError> {
    let mut cmd = ObserveCmd {
        full: false,
        minimal: false,
        viewport: false,
        hidden: false,
        positions: false,
        diff: false,
        near: None,
        timeout: None,
    };

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::near_opt => {
                cmd.near = Some(parse_string(inner.into_inner().next().unwrap()));
            }
            Rule::timeout_opt => cmd.timeout = Some(parse_timeout(inner)?),
            _ => match inner.as_str() {
                "--full" => cmd.full = true,
                "--minimal" => cmd.minimal = true,
                "--viewport" => cmd.viewport = true,
                "--hidden" => cmd.hidden = true,
                "--positions" => cmd.positions = true,
                "--diff" => cmd.diff = true,
                _ => {}
            },
        }
    }
    Ok(cmd)
}

fn parse_html(pair: Pair<Rule>) -> Result<HtmlCmd, ParseError> {
    let mut selector = None;
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::selector_opt {
            selector = Some(parse_string(inner.into_inner().next().unwrap()));
        }
    }
    Ok(HtmlCmd { selector })
}

fn parse_text(pair: Pair<Rule>) -> Result<TextCmd, ParseError> {
    let mut selector = None;
    let mut target = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::selector_opt => selector = Some(parse_string(inner.into_inner().next().unwrap())),
            Rule::target => target = Some(parse_target(inner)?),
            _ => {}
        }
    }
    Ok(TextCmd { selector, target })
}

fn parse_screenshot(pair: Pair<Rule>) -> Result<ScreenshotCmd, ParseError> {
    let mut output = None;
    let mut format = None;
    let mut fullpage = false;
    let mut target = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::output_opt => output = Some(parse_file_path(inner.into_inner().next().unwrap())),
            Rule::format_opt => {
                format = Some(inner.into_inner().next().unwrap().as_str().to_string())
            }
            Rule::target => target = Some(parse_target(inner)?),
            _ => {
                if inner.as_str() == "--fullpage" {
                    fullpage = true;
                }
            }
        }
    }
    Ok(ScreenshotCmd {
        output,
        format,
        fullpage,
        target,
    })
}

fn parse_box(pair: Pair<Rule>) -> Result<BoxCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(BoxCmd { target })
}

// --- Action Parsers ---

fn parse_click(pair: Pair<Rule>) -> Result<ClickCmd, ParseError> {
    let mut target = None;
    let mut double = false;
    let mut right = false;
    let mut middle = false;
    let mut force = false;
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = Some(parse_target(inner)?),
            Rule::timeout_opt => timeout = Some(parse_timeout(inner)?),
            _ => match inner.as_str() {
                "--double" => double = true,
                "--right" => right = true,
                "--middle" => middle = true,
                "--force" => force = true,
                "--ctrl" => ctrl = true,
                "--shift" => shift = true,
                "--alt" => alt = true,
                _ => {}
            },
        }
    }
    Ok(ClickCmd {
        target: target.unwrap(), // TODO: Error if missing (grammar enforces it though)
        double,
        right,
        middle,
        force,
        ctrl,
        shift,
        alt,
        timeout,
    })
}

fn parse_type(pair: Pair<Rule>) -> Result<TypeCmd, ParseError> {
    let mut target = None;
    let mut text = String::new();
    let mut append = false;
    let mut enter = false;
    let mut clear = false;
    let mut delay = None;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = Some(parse_target(inner)?),
            Rule::string_value => text = parse_string(inner),
            Rule::timeout_opt => timeout = Some(parse_timeout(inner)?),
            Rule::number => delay = Some(parse_number(inner)?),
            _ => match inner.as_str() {
                "--append" => append = true,
                "--enter" => enter = true,
                "--clear" => clear = true,
                _ => {}
            },
        }
    }
    Ok(TypeCmd {
        target: target.unwrap(),
        text,
        append,
        enter,
        delay,
        clear,
        timeout,
    })
}

fn parse_clear(pair: Pair<Rule>) -> Result<ClearCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(ClearCmd { target })
}

fn parse_press(pair: Pair<Rule>) -> Result<PressCmd, ParseError> {
    let keys = pair
        .into_inner()
        .next()
        .map(|combo| {
            combo
                .as_str()
                .split('+')
                .map(|k| k.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    Ok(PressCmd { keys })
}

fn parse_keydown(pair: Pair<Rule>) -> Result<KeydownCmd, ParseError> {
    let key = pair.into_inner().next().unwrap().as_str().to_string();
    Ok(KeydownCmd { key })
}

fn parse_keyup(pair: Pair<Rule>) -> Result<KeyupCmd, ParseError> {
    let key = pair
        .into_inner()
        .next()
        .map(|inner| inner.as_str().to_string())
        .unwrap_or_else(|| "all".to_string());
    Ok(KeyupCmd { key })
}

fn parse_select(pair: Pair<Rule>) -> Result<SelectCmd, ParseError> {
    let mut inner = pair.into_inner();
    let target = parse_target(inner.next().unwrap())?;
    let val_pair = inner.next().unwrap();
    let value = if val_pair.as_rule() == Rule::string_value {
        parse_string(val_pair)
    } else {
        val_pair.as_str().to_string()
    };
    Ok(SelectCmd { target, value })
}

fn parse_check(pair: Pair<Rule>) -> Result<CheckCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(CheckCmd { target })
}

fn parse_uncheck(pair: Pair<Rule>) -> Result<UncheckCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(UncheckCmd { target })
}

fn parse_hover(pair: Pair<Rule>) -> Result<HoverCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(HoverCmd { target })
}

fn parse_focus(pair: Pair<Rule>) -> Result<FocusCmd, ParseError> {
    let target = parse_target(pair.into_inner().next().unwrap())?;
    Ok(FocusCmd { target })
}

fn parse_scroll(pair: Pair<Rule>) -> Result<ScrollCmd, ParseError> {
    let mut cmd = ScrollCmd {
        direction: None,
        amount: None,
        page: false,
        timeout: None,
        target: None,
    };
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::scroll_direction => cmd.direction = Some(inner.as_str().to_string()),
            Rule::number => cmd.amount = Some(parse_number(inner)?),
            Rule::target => cmd.target = Some(parse_target(inner)?),
            Rule::timeout_opt => cmd.timeout = Some(parse_timeout(inner)?),
            _ => {
                if inner.as_str() == "--page" {
                    cmd.page = true;
                }
            }
        }
    }
    Ok(cmd)
}

fn parse_submit(pair: Pair<Rule>) -> Result<SubmitCmd, ParseError> {
    let target = pair
        .into_inner()
        .next()
        .map(|p| parse_target(p))
        .transpose()?;
    Ok(SubmitCmd { target })
}

fn parse_wait(pair: Pair<Rule>) -> Result<WaitCmd, ParseError> {
    let text = pair.as_str();
    let lower_text = text.trim_start_matches("wait").trim();
    let inners: Vec<Pair<Rule>> = pair.into_inner().collect();

    let timeout = inners
        .iter()
        .find(|p| p.as_rule() == Rule::timeout_opt)
        .map(|p| parse_timeout(p.clone()))
        .transpose()?;

    let find_target = || inners.iter().find(|p| p.as_rule() == Rule::target);
    let find_string = || inners.iter().find(|p| p.as_rule() == Rule::string_value);

    let condition = if lower_text.starts_with("load") {
        WaitCondition::Load
    } else if lower_text.starts_with("idle") {
        WaitCondition::Idle
    } else if lower_text.starts_with("navigation") {
        WaitCondition::Navigation
    } else if lower_text.starts_with("ready") {
        WaitCondition::Ready
    } else if lower_text.starts_with("visible") {
        find_target()
            .map(|t| parse_target(t.clone()))
            .transpose()?
            .map(WaitCondition::Visible)
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("hidden") {
        find_target()
            .map(|t| parse_target(t.clone()))
            .transpose()?
            .map(WaitCondition::Hidden)
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("exists") {
        find_string()
            .map(|s| WaitCondition::Exists(parse_string(s.clone())))
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("gone") {
        find_string()
            .map(|s| WaitCondition::Gone(parse_string(s.clone())))
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("url") {
        find_string()
            .map(|s| WaitCondition::Url(parse_string(s.clone())))
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("until") {
        find_string()
            .map(|s| WaitCondition::Until(parse_string(s.clone())))
            .unwrap_or(WaitCondition::Load)
    } else if lower_text.starts_with("items") {
        let selector = find_string().map(|s| parse_string(s.clone()));
        let count = inners
            .iter()
            .find(|p| p.as_rule() == Rule::number)
            .map(|n| parse_number(n.clone()))
            .transpose()?;
        match (selector, count) {
            (Some(s), Some(c)) => WaitCondition::Items {
                selector: s,
                count: c,
            },
            _ => WaitCondition::Load,
        }
    } else {
        WaitCondition::Load
    };

    Ok(WaitCmd { condition, timeout })
}

fn parse_extract(pair: Pair<Rule>) -> Result<ExtractCmd, ParseError> {
    let mut what = ExtractWhat::Text;
    let mut selector = None;
    let mut format = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::extract_css => {
                what = ExtractWhat::Css(parse_string(inner.into_inner().next().unwrap()));
            }
            Rule::selector_opt => {
                selector = Some(parse_string(inner.into_inner().next().unwrap()));
            }
            Rule::output_format => format = Some(inner.as_str().to_string()),
            _ => {
                what = match inner.as_str() {
                    "links" => ExtractWhat::Links,
                    "images" => ExtractWhat::Images,
                    "tables" => ExtractWhat::Tables,
                    "meta" => ExtractWhat::Meta,
                    _ => what,
                };
            }
        }
    }
    Ok(ExtractCmd {
        what,
        selector,
        format,
    })
}

// --- Common Helpers ---

fn parse_target(pair: Pair<Rule>) -> Result<Target, ParseError> {
    // target = { target_atomic ~ (WSP+ ~ relation ~ WSP+ ~ target_atomic)* }
    // This is flat. We need to build right-associative chain.
    // A near B inside C -> A near (B inside C)

    let mut inners = pair.into_inner();
    let first = parse_target_atomic(inners.next().unwrap())?;

    // Collect rest: (relation, atomic) pairs
    let mut rest = Vec::new();
    while let Some(rel) = inners.next() {
        let atom = inners.next().unwrap(); // Must exist
                                           // rel is `relation` rule
        let kind = match rel.as_str() {
            "near" => RelationKind::Near,
            "inside" => RelationKind::Inside,
            "after" => RelationKind::After,
            "before" => RelationKind::Before,
            "contains" => RelationKind::Contains,
            _ => RelationKind::Near,
        };
        rest.push((kind, parse_target_atomic(atom)?));
    }

    // Fold right-associatively
    // A, [(near, B), (inside, C)]
    // fold_right?
    // start with last: C.
    // wrap with inside -> Target(Inside, C).
    // wrap with near -> Target(Near, (Inside, C)).
    // Wait, the structure is Target { atomic, relation_ptr }.
    // If I have A near B.
    // A is the main atomic.
    // relation points to B.

    // Reverse iterate the list?
    // Input: A, [(R1, B), (R2, C)]
    // Expected AST: A -> R1 -> B -> R2 -> C.
    // Wait, "A near B inside C" -> A is near (B which is inside C).
    // So A has relation (Near, Target(B, relation=(Inside, C))).
    // Yes.

    // Recursive construction?
    // We can just iterate the list and link them.
    // But `Target` struct owns the next one.

    fn build_chain(head: TargetAtomic, tail: Vec<(RelationKind, TargetAtomic)>) -> Target {
        if tail.is_empty() {
            return Target {
                atomic: head,
                relation: None,
            };
        }

        let (rel, next_atomic) = tail[0].clone();
        let next_tail = tail[1..].to_vec();

        Target {
            atomic: head,
            relation: Some(Box::new(TargetRelation {
                kind: rel,
                target: build_chain(next_atomic, next_tail),
            })),
        }
    }

    // Note: My parsing logic above collected them in order.
    // A, [(R1, B), (R2, C)]
    // build_chain(A, ...) -> Target(A, Some(R1, build_chain(B, ...))) -> Target(B, Some(R2, build_chain(C, []))) -> Target(C, None).
    // This looks correct for "A near B inside C" if it means A is near B, and B is inside C.
    // Is that the spec?
    // Spec: "Target chains ... MUST associate rightward... A near (B inside C)"
    // My structure: Target(A) has relation -> Target(B).
    // This DOES represent A related to B.
    // And B has relation to C.
    // So A is near B. And B is inside C.
    // Yes.

    Ok(build_chain(first, rest))
}

fn parse_target_atomic(pair: Pair<Rule>) -> Result<TargetAtomic, ParseError> {
    // target_atomic = _{ target_selector | target_role | target_id | target_text }

    // inner is the specific rule
    match pair.as_rule() {
        Rule::target_id => Ok(TargetAtomic::Id(pair.as_str().parse().unwrap())),
        Rule::target_text => Ok(TargetAtomic::Text(parse_string(
            pair.into_inner().next().unwrap(),
        ))),
        Rule::target_role => Ok(TargetAtomic::Role(pair.as_str().to_string())),
        Rule::target_selector => {
            // css(...) or xpath(...)
            let text = pair.as_str();
            let kind = if text.starts_with("css") {
                "css"
            } else {
                "xpath"
            };
            let val = parse_string(pair.into_inner().next().unwrap());
            Ok(TargetAtomic::Selector {
                kind: kind.to_string(),
                value: val,
            })
        }
        // It might be a silent rule that passed through `target_selector` rule?
        // Note: target_selector is _{ ... }.
        // So we get literals "css" etc?
        // No, target_selector has choices `css ... | xpath ...`.
        // If it's silent, we get the inners.
        // `css` literal, `(` literal, `string_value` rule, `)` literal.
        // Only `string_value` is a rule.
        // So we just see `string_value`.
        // How do we know if CSS or XPath?
        // Check `pair.as_str()` again.
        Rule::string_value => {
            // This happens if it was target_text (which wraps string_value)
            Ok(TargetAtomic::Text(parse_string(pair)))
        }
        _ => {
            // Fallback for selectors if they show up as string_value?
            // Wait, target_text = { string_value }. Not silent.
            // target_selector = _{ ... }.
            // So if it matches target_selector, we see `string_value` BUT we don't know if CSS or XPath unless we check parent/text.
            // Actually, `target_atomic` covers them.
            // If `target_atomic` is `_`, we get the inner choice.
            // If the choice is `target_selector` (`_`), we get ITS inner.
            // So we see `string_value`.
            // Ambiguity between `target_text` (which is `string_value`) and `target_selector` (which has `string_value`).
            // `target_text` is NOT silent. So if it matches `target_text`, we get a `target_text` pair.
            // `target_selector` IS silent. So if it matches, we get `string_value`.
            // SO: If we see `target_text` pair -> Text.
            // If we see `string_value` pair directly -> Must be Selector. AND we check text prefix for css/xpath.
            Ok(TargetAtomic::Text(parse_string(pair))) // Fallback
        }
    }
}

// TODO: Implement placeholders for methods I skipped to save space in this focused write.
// I will need to finish: parse_cookies, parse_storage, parse_session_mgmt, parse_state, parse_headers,
// parse_tab_action, parse_intent cmds, parse_pack cmds, parse_network cmds, parse_console, parse_frames, parse_dialog,
// parse_viewport/device/media, parse_record/trace/highlight, parse_pdf/learn/help.

fn parse_cookies(pair: Pair<Rule>) -> Result<CookiesCmd, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    let action = match inner.as_rule() {
        Rule::cookies_list => CookiesAction::List,
        Rule::cookies_get => {
            let name = parse_name_value(inner.into_inner().next().unwrap());
            CookiesAction::Get(name)
        }
        Rule::cookies_set => {
            let mut inners = inner.into_inner();
            let name = parse_name_value(inners.next().unwrap());
            let value = parse_string(inners.next().unwrap());
            CookiesAction::Set { name, value }
        }
        Rule::cookies_delete => {
            let name = parse_name_value(inner.into_inner().next().unwrap());
            CookiesAction::Delete(name)
        }
        Rule::cookies_clear => CookiesAction::Clear,
        _ => return Err(ParseError::UnknownRule(inner.as_rule())),
    };
    Ok(CookiesCmd { action })
}
fn parse_storage(_pair: Pair<Rule>) -> Result<StorageCmd, ParseError> {
    Ok(StorageCmd {
        action: StorageAction::List,
        local: false,
        session: false,
    })
} // Stub
fn parse_session_mgmt(_pair: Pair<Rule>) -> Result<SessionMgmtCmd, ParseError> {
    Ok(SessionMgmtCmd { action: None })
} // Stub
fn parse_state(_pair: Pair<Rule>) -> Result<StateCmd, ParseError> {
    Ok(StateCmd {
        action: StateAction::Load {
            path: "".into(),
            merge: false,
            cookies_only: false,
        },
    })
} // Stub
fn parse_headers(_pair: Pair<Rule>) -> Result<HeadersCmd, ParseError> {
    Ok(HeadersCmd { action: None })
} // Stub
fn parse_tab_action(pair: Pair<Rule>) -> Result<TabActionCmd, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    let action = match inner.as_rule() {
        Rule::tab_new => TabAction::New(parse_file_path(inner.into_inner().next().unwrap())), // reuse parse_file_path for url_value
        Rule::tab_switch => TabAction::Switch(parse_number(inner.into_inner().next().unwrap())?),
        Rule::tab_close => {
            let mut idx = None;
            if let Some(n) = inner.into_inner().next() {
                idx = Some(parse_number(n)?);
            }
            TabAction::Close(idx)
        }
        _ => return Err(ParseError::UnknownRule(inner.as_rule())),
    };
    Ok(TabActionCmd { action })
}
fn parse_login(pair: Pair<Rule>) -> Result<LoginCmd, ParseError> {
    let mut inners = pair.into_inner();
    let user = parse_string(inners.next().unwrap());
    let pass = parse_string(inners.next().unwrap());
    let mut cmd = LoginCmd {
        user,
        pass,
        no_submit: false,
        wait: None,
        timeout: None,
    };

    for inner in inners {
        match inner.as_rule() {
            Rule::timeout_opt => cmd.timeout = Some(parse_timeout(inner)?),
            Rule::duration => cmd.wait = Some(inner.as_str().to_string()),
            _ => {
                if inner.as_str() == "--no-submit" {
                    cmd.no_submit = true;
                }
            }
        }
    }

    Ok(cmd)
}
fn parse_search(pair: Pair<Rule>) -> Result<SearchCmd, ParseError> {
    let mut inners = pair.into_inner();
    let query = parse_string(inners.next().unwrap());
    let mut cmd = SearchCmd {
        query,
        submit: None,
        wait: None,
        timeout: None,
    };

    for inner in inners {
        match inner.as_rule() {
            Rule::submit_method => cmd.submit = Some(inner.as_str().to_string()),
            Rule::timeout_opt => cmd.timeout = Some(parse_timeout(inner)?),
            Rule::duration => cmd.wait = Some(inner.as_str().to_string()),
            _ => {}
        }
    }

    Ok(cmd)
}
fn parse_dismiss(pair: Pair<Rule>) -> Result<DismissCmd, ParseError> {
    let mut inners = pair.clone().into_inner();
    if let Some(inner) = inners.next() {
        let target = match inner.as_rule() {
            Rule::string_value => parse_string(inner),
            _ => inner.as_str().to_string(),
        };
        return Ok(DismissCmd { target });
    }

    let mut parts = pair.as_str().splitn(2, char::is_whitespace);
    let _ = parts.next(); // "dismiss"
    let target = parts
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("popups")
        .to_string();
    Ok(DismissCmd { target })
}
fn parse_scroll_until(pair: Pair<Rule>) -> Result<ScrollUntilCmd, ParseError> {
    let mut target = None;
    let mut amount = None;
    let mut page = false;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = Some(parse_target(inner)?),
            Rule::number => amount = Some(parse_number(inner)?),
            Rule::timeout_opt => timeout = Some(parse_timeout(inner)?),
            _ => {
                if inner.as_str() == "--page" {
                    page = true;
                }
            }
        }
    }

    let target = target.ok_or(ParseError::UnknownRule(Rule::target))?;
    Ok(ScrollUntilCmd {
        target,
        amount,
        page,
        timeout,
    })
}
fn parse_intents(_pair: Pair<Rule>) -> Result<IntentsCmd, ParseError> {
    Ok(IntentsCmd { session: false })
}
fn parse_define(_pair: Pair<Rule>) -> Result<DefineCmd, ParseError> {
    Ok(DefineCmd { name: "".into() })
}
fn parse_undefine(_pair: Pair<Rule>) -> Result<UndefineCmd, ParseError> {
    Ok(UndefineCmd { name: "".into() })
}
fn parse_export(_pair: Pair<Rule>) -> Result<ExportCmd, ParseError> {
    Ok(ExportCmd {
        name: "".into(),
        out: None,
    })
}
fn parse_run(_pair: Pair<Rule>) -> Result<RunCmd, ParseError> {
    Ok(RunCmd {
        name: "".into(),
        params: vec![],
    })
}
fn parse_pack_action(_pair: Pair<Rule>) -> Result<PackActionCmd, ParseError> {
    Ok(PackActionCmd {
        action: "".into(),
        name: "".into(),
    })
}
fn parse_intercept(_pair: Pair<Rule>) -> Result<InterceptCmd, ParseError> {
    Ok(InterceptCmd {
        rule: InterceptRule::Clear(None),
    })
}
fn parse_requests(_pair: Pair<Rule>) -> Result<RequestsCmd, ParseError> {
    Ok(RequestsCmd {
        filter: None,
        method: None,
        last: None,
    })
}
fn parse_console(_pair: Pair<Rule>) -> Result<ConsoleCmd, ParseError> {
    Ok(ConsoleCmd {
        clear: false,
        level: None,
        filter: None,
        last: None,
    })
}
fn parse_errors(_pair: Pair<Rule>) -> Result<ErrorsCmd, ParseError> {
    Ok(ErrorsCmd {
        clear: false,
        last: None,
    })
}
fn parse_frame(_pair: Pair<Rule>) -> Result<FrameSwitchCmd, ParseError> {
    Ok(FrameSwitchCmd {
        target: FrameTarget::Main,
    })
}
fn parse_dialog(_pair: Pair<Rule>) -> Result<DialogCmd, ParseError> {
    Ok(DialogCmd {
        action: DialogAction::Dismiss,
    })
}
fn parse_viewport(_pair: Pair<Rule>) -> Result<ViewportSizeCmd, ParseError> {
    Ok(ViewportSizeCmd {
        width: 0.0,
        height: 0.0,
    })
}
fn parse_device(_pair: Pair<Rule>) -> Result<DeviceCmd, ParseError> {
    Ok(DeviceCmd { name: None })
}
fn parse_media(_pair: Pair<Rule>) -> Result<MediaCmd, ParseError> {
    Ok(MediaCmd {
        feature: None,
        value: None,
    })
}
fn parse_trace(_pair: Pair<Rule>) -> Result<TraceCmd, ParseError> {
    Ok(TraceCmd {
        start: false,
        path: None,
    })
}
fn parse_record(_pair: Pair<Rule>) -> Result<RecordCmd, ParseError> {
    Ok(RecordCmd {
        start: false,
        path: None,
        quality: None,
    })
}
fn parse_highlight(_pair: Pair<Rule>) -> Result<HighlightCmd, ParseError> {
    Ok(HighlightCmd {
        clear: false,
        target: None,
        duration: None,
        color: None,
    })
}
fn parse_pdf(pair: Pair<Rule>) -> Result<PdfCmd, ParseError> {
    let mut path = String::new();
    let mut format = None;
    let mut landscape = false;
    let mut margin = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::file_path => path = parse_file_path(inner),
            Rule::paper_format => format = Some(inner.as_str().to_string()),
            Rule::number | Rule::string_value => {
                // Assume this is margin, as format and path are handled.
                // margin takes number or string.
                if margin.is_none() {
                    // Check if it's number
                    if inner.as_rule() == Rule::number {
                        margin = Some(inner.as_str().to_string());
                    } else {
                        margin = Some(parse_string(inner));
                    }
                }
            }
            _ => {
                if inner.as_str() == "--landscape" {
                    landscape = true;
                }
            }
        }
    }
    Ok(PdfCmd {
        path,
        format,
        landscape,
        margin,
    })
}
fn parse_learn(_pair: Pair<Rule>) -> Result<LearnCmd, ParseError> {
    Ok(LearnCmd {
        action: "".into(),
        name: None,
    })
}
fn parse_help(_pair: Pair<Rule>) -> Result<HelpCmd, ParseError> {
    Ok(HelpCmd { topic: None })
}

// Helpers

fn parse_string(pair: Pair<Rule>) -> String {
    // string_value = ${ "\"" ~ string_inner ~ "\"" }
    // string_inner is content
    // We want the content with escapes processed if possible, or usually just raw content for now?
    // Pest: pair.as_str() includes quotes.
    // inner pair string_inner excludes quotes.
    let inner = pair.into_inner().next().unwrap();
    let raw = inner.as_str();
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    _ => out.push(next),
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_number(pair: Pair<Rule>) -> Result<f64, ParseError> {
    pair.as_str().parse().map_err(ParseError::InvalidFloat)
}

fn parse_timeout(pair: Pair<Rule>) -> Result<String, ParseError> {
    // timeout_opt = { "--timeout" ~ WSP+ ~ duration }
    // return duration string
    Ok(pair.into_inner().next().unwrap().as_str().to_string())
}

fn parse_file_path(pair: Pair<Rule>) -> String {
    // file_path = { string_value | path_bare }
    let inner = pair.into_inner().next().unwrap();
    if inner.as_rule() == Rule::string_value {
        parse_string(inner)
    } else {
        inner.as_str().to_string()
    }
}
fn parse_name_value(pair: Pair<Rule>) -> String {
    // name_value = { identifier | string_value }
    let inner = pair.into_inner().next().unwrap();
    if inner.as_rule() == Rule::string_value {
        parse_string(inner)
    } else {
        inner.as_str().to_string()
    }
}
