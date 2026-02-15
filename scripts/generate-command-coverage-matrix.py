#!/usr/bin/env python3
"""Generate docs/command-coverage-matrix.md from parser/translator/executor sources."""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
AST_PATH = ROOT / "crates/oryn-core/src/ast.rs"
PARSER_PATH = ROOT / "crates/oryn-core/src/parser.rs"
TRANSLATOR_PATH = ROOT / "crates/oryn-core/src/translator.rs"
OUTPUT_PATH = ROOT / "docs/command-coverage-matrix.md"


@dataclass
class CommandCoverage:
    command: str
    parser_status: str
    translator_status: str
    action: str
    executor_status: str
    note: str
    parse_fn: str


NOTES = {
    "Goto": "`--headers/--timeout` parse but are currently not applied in translation",
    "Refresh": "`--hard` parses, but executor currently ignores hard/soft distinction",
    "Observe": "`minimal/positions/timeout` parsed but not used in translation",
    "Text": "`target` parsed but translator uses only `selector`",
    "Screenshot": "`target` parsed but translator sets `selector: None`",
    "Click": "`--ctrl/--shift/--alt` parsed but translator sends empty modifiers",
    "Type": "`--append/--timeout` parsed but currently not applied in translation",
    "Scroll": "`--timeout` parsed but currently not applied in translation",
    "Wait": "`wait url \"...\"` downgraded to generic `navigation`; `ready` maps to unsupported condition",
    "Login": "`--no-submit/--wait/--timeout` parsed but current translation ignores these options",
    "Search": "`--submit/--wait/--timeout` parsed but current translation ignores these options",
    "Cookies": (
        "Executor handles list/get/set/delete; `clear` is `NotImplemented`; "
        "backend `set_cookie` defaults to `NotSupported`"
    ),
    "Tabs": "Executor handles only tab action `list`",
    "Tab": "Translator emits `new/switch/close`; executor handles only `list`",
    "Pdf": "Only headless backend implements `pdf`",
    "Exit": "Not translated; REPL exits via raw `exit`/`quit` checks in CLI",
}

PARTIAL_EXECUTOR = {"Cookies", "Tabs", "Pdf"}
STUBBED_EXECUTOR = {"Tab"}


def parse_commands(ast_text: str) -> list[str]:
    match = re.search(r"pub enum Command \{(.*?)\n\}", ast_text, re.S)
    if not match:
        raise RuntimeError("Could not find `pub enum Command` in ast.rs")

    commands: list[str] = []
    for raw_line in match.group(1).splitlines():
        line = raw_line.strip().rstrip(",")
        cmd_match = re.match(r"^([A-Z][A-Za-z0-9_]*)", line)
        if cmd_match:
            commands.append(cmd_match.group(1))
    return commands


def parse_parser_mappings(parser_text: str) -> dict[str, str]:
    mappings: dict[str, str] = {}
    pattern = re.compile(
        r"Rule::[a-z0-9_]+\s*=>\s*Ok\(Command::([A-Za-z0-9_]+)"
        r"(?:\((parse_[a-z0-9_]+)\(pair\)\?\))?\)"
    )
    for line in parser_text.splitlines():
        match = pattern.search(line)
        if match:
            mappings[match.group(1)] = match.group(2) or "const"
    return mappings


def parse_stub_parse_functions(parser_text: str) -> set[str]:
    return set(re.findall(r"fn\s+(parse_[a-z0-9_]+)\(_pair:", parser_text))


def parse_translator_blocks(translator_text: str) -> dict[str, str]:
    blocks: dict[str, str] = {}
    current_cmd: str | None = None
    current_lines: list[str] = []

    for line in translator_text.splitlines():
        if re.match(r"^\s{8}Command::[A-Za-z0-9_]+", line):
            cmd = re.search(r"Command::([A-Za-z0-9_]+)", line)
            if cmd is None:
                continue
            if current_cmd is not None:
                blocks[current_cmd] = "\n".join(current_lines)
            current_cmd = cmd.group(1)
            current_lines = [line]
            continue

        if current_cmd is None:
            continue

        if re.match(r"^\s{8}_\s*=>", line):
            blocks[current_cmd] = "\n".join(current_lines)
            current_cmd = None
            current_lines = []
            continue

        current_lines.append(line)

    if current_cmd is not None:
        blocks[current_cmd] = "\n".join(current_lines)

    return blocks


def parse_translator_actions(blocks: dict[str, str]) -> dict[str, str]:
    action_map: dict[str, str] = {}
    for command, block in blocks.items():
        full_match = re.search(r"Action::([A-Za-z]+)\((\w+)::([A-Za-z0-9_]+)", block)
        if full_match:
            action_map[command] = (
                f"{full_match.group(1)}::{full_match.group(2)}::{full_match.group(3)}"
            )
            continue

        partial_match = re.search(r"Action::([A-Za-z]+)\(", block)
        if partial_match:
            action_map[command] = f"{partial_match.group(1)}::?"
            continue

        action_map[command] = "-"
    return action_map


def executor_status_for(command: str, translator_status: str) -> str:
    if translator_status == "stubbed":
        return "-"
    if command in STUBBED_EXECUTOR:
        return "stubbed"
    if command in PARTIAL_EXECUTOR:
        return "partial"
    return "implemented"


def build_rows() -> list[CommandCoverage]:
    ast_text = AST_PATH.read_text()
    parser_text = PARSER_PATH.read_text()
    translator_text = TRANSLATOR_PATH.read_text()

    commands = parse_commands(ast_text)
    parser_map = parse_parser_mappings(parser_text)
    parser_stub_fns = parse_stub_parse_functions(parser_text)
    translator_blocks = parse_translator_blocks(translator_text)
    translator_action_map = parse_translator_actions(translator_blocks)

    rows: list[CommandCoverage] = []
    for command in commands:
        parse_fn = parser_map.get(command, "-")
        parser_status = "stubbed" if parse_fn in parser_stub_fns else "implemented"

        if command in translator_action_map:
            translator_status = "implemented"
            action = f"`{translator_action_map[command]}`"
        else:
            translator_status = "stubbed"
            action = "-"

        rows.append(
            CommandCoverage(
                command=command,
                parser_status=parser_status,
                translator_status=translator_status,
                action=action,
                executor_status=executor_status_for(command, translator_status),
                note=NOTES.get(command, ""),
                parse_fn=parse_fn,
            )
        )

    return rows


def build_summary(rows: list[CommandCoverage]) -> dict[str, int]:
    summary = {
        "total": len(rows),
        "parser_implemented": 0,
        "parser_stubbed": 0,
        "translator_implemented": 0,
        "translator_stubbed": 0,
        "end_to_end_implemented": 0,
        "partial": 0,
        "blocked": 0,
    }

    for row in rows:
        summary["parser_implemented" if row.parser_status == "implemented" else "parser_stubbed"] += 1
        summary[
            "translator_implemented"
            if row.translator_status == "implemented"
            else "translator_stubbed"
        ] += 1

        if (
            row.parser_status == "implemented"
            and row.translator_status == "implemented"
            and row.executor_status == "implemented"
        ):
            summary["end_to_end_implemented"] += 1
        elif (
            row.parser_status == "implemented"
            and row.translator_status == "implemented"
            and row.executor_status == "partial"
        ):
            summary["partial"] += 1
        else:
            summary["blocked"] += 1

    return summary


def render_markdown(rows: list[CommandCoverage], summary: dict[str, int]) -> str:
    generated = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")

    lines = [
        "# Command Coverage Matrix",
        "",
        "Generated from:",
        "- `crates/oryn-core/src/ast.rs`",
        "- `crates/oryn-core/src/parser.rs`",
        "- `crates/oryn-core/src/translator.rs`",
        "- `crates/oryn-engine/src/executor.rs`",
        "",
        f"Generated at (UTC): `{generated}`",
        "",
        "Regenerate:",
        "- `python scripts/generate-command-coverage-matrix.py`",
        "",
        "Legend:",
        "- `implemented`: wired through that stage",
        "- `partial`: wired but behavior is limited",
        "- `stubbed`: placeholder/default or unsupported",
        "",
        "Snapshot:",
        f"- Total AST commands: `{summary['total']}`",
        f"- Parser implemented: `{summary['parser_implemented']}`",
        f"- Parser stubbed: `{summary['parser_stubbed']}`",
        f"- Translator implemented: `{summary['translator_implemented']}`",
        f"- Translator stubbed: `{summary['translator_stubbed']}`",
        f"- End-to-end implemented: `{summary['end_to_end_implemented']}`",
        f"- Partial in executor/backend path: `{summary['partial']}`",
        f"- Blocked before execution: `{summary['blocked']}`",
        "",
        "| Command | Parser | Translator | Action | Executor | Note |",
        "|---|---|---|---|---|---|",
    ]

    for row in rows:
        lines.append(
            f"| {row.command} | {row.parser_status} | {row.translator_status} | "
            f"{row.action} | {row.executor_status} | {row.note} |"
        )

    return "\n".join(lines) + "\n"


def main() -> None:
    rows = build_rows()
    summary = build_summary(rows)
    markdown = render_markdown(rows, summary)
    OUTPUT_PATH.write_text(markdown)
    print(f"Wrote {OUTPUT_PATH.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
