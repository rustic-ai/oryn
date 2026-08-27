#!/usr/bin/env python3
"""Run the seeded, OIL-only MiniWoB oracle for native/Chromium parity."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import math
import re
import subprocess
import time
import urllib.request
from dataclasses import asdict, dataclass, is_dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from intentgym.core.oryn import OrynInterface, OrynObservation, OrynResult

TASKS = (
    "click-button",
    "click-link",
    "click-option",
    "enter-text",
    "focus-text",
    "choose-date",
    "login-user",
    "search-engine",
)
SEEDS = (17, 42, 73)
MONTHS = (
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
)
ALLOWED_NATIVE_LIMITATIONS = {"rendered_layout_and_paint"}


@dataclass
class OracleTurn:
    observation: dict[str, Any]
    oil: str
    resolution: dict[str, Any]
    result: dict[str, Any]
    effects: list[dict[str, Any]]
    trace_slice: list[dict[str, Any]]
    task_reward: float | None
    failure_domain_reason_code: str | None


def _json_value(raw: str) -> Any:
    for line in raw.splitlines():
        if line.startswith("Value: "):
            try:
                payload = json.loads(line.removeprefix("Value: "))
            except json.JSONDecodeError:
                pass
            else:
                if isinstance(payload, dict) and "text" in payload:
                    return payload["text"]
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict) and "value" in payload:
            return payload["value"]
    return raw


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _canonical_hash(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _run_fingerprint(
    args: argparse.Namespace, domains: tuple[str, ...]
) -> dict[str, Any]:
    runtime = json.loads(
        subprocess.check_output(
            [str(args.oryn_binary), "native", "--runtime-info"], text=True
        )
    )
    if "native" in domains and (
        runtime.get("native_v8") is not True
        or runtime.get("build_profile") != "release"
    ):
        raise RuntimeError(
            "deterministic native evidence requires the release V8 binary"
        )
    with urllib.request.urlopen(
        args.miniwob_url.rstrip("/") + "/.well-known/oryn-g2-fixture.json",
        timeout=10,
    ) as response:
        fixture = json.load(response)
    chromium_version = None
    if "chromium" in domains:
        chromium_version = subprocess.check_output(
            [str(args.chrome_binary), "--version"], text=True
        ).strip()
    fingerprint = {
        "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=args.repo_root, text=True
        ).strip(),
        "binary": {
            "sha256": _sha256(args.oryn_binary),
            "bytes": args.oryn_binary.stat().st_size,
            "runtime": runtime,
        },
        "miniwob": fixture,
        "chromium_version": chromium_version,
        "oracle": {
            "id": "oil_deterministic_oracle_v1",
            "sha256": _sha256(Path(__file__).resolve()),
        },
    }
    fingerprint["sha256"] = _canonical_hash(fingerprint)
    return fingerprint


def _json_trace(raw: str) -> list[dict[str, Any]]:
    for line in raw.splitlines():
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict) and payload.get("kind") == "trace":
            return list(payload.get("events") or [])
    return []


def _serializable(value: Any) -> Any:
    if is_dataclass(value):
        return asdict(value)
    if isinstance(value, dict):
        return {str(key): _serializable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_serializable(item) for item in value]
    return value


def _observation_record(observation: OrynObservation | None) -> dict[str, Any]:
    if observation is None:
        return {}
    return {
        "contract_version": observation.contract_version,
        "revision": observation.revision,
        "document_generation": observation.document_generation,
        "url": observation.url,
        "title": observation.title,
        "elements": _serializable(observation.elements),
        "capabilities": _serializable(observation.capabilities),
        "diagnostics": observation.diagnostics,
        "utf8_bytes": observation.byte_count or len(observation.raw.encode("utf-8")),
        "execution_domain": observation.execution_domain,
    }


def _actionable(observation: OrynObservation, action: str) -> list[dict[str, Any]]:
    if any("actions" in element for element in observation.elements):
        return [
            element
            for element in observation.elements
            if action in (element.get("actions") or [])
        ]
    inferred = []
    for element in observation.elements:
        element_type = element.get("type")
        role = element.get("role")
        name = element.get("text")
        supported = False
        if action == "click":
            supported = (
                element_type
                in {"button", "link", "radio", "checkbox", "input", "textarea"}
                or role in {"button", "link", "radio", "checkbox"}
                or name == "START"
            )
        elif action in {"type", "focus"}:
            supported = element_type in {"input", "textarea"} or role in {
                "textbox",
                "email",
                "password",
            }
        elif action == "check":
            supported = element_type in {"radio", "checkbox"} or role in {
                "radio",
                "checkbox",
            }
        if supported:
            inferred.append(element)
    return inferred


def _role_matches(element: dict[str, Any], expected: str) -> bool:
    element_type = element.get("type")
    role = element.get("role")
    if expected == "textbox":
        return element_type in {"input", "textarea"} or role in {
            "textbox",
            "email",
            "password",
        }
    if expected == "button":
        return element_type == "button" or role == "button"
    if expected == "radio":
        return element_type == "radio" or role == "radio"
    if expected == "link":
        return element_type == "link" or role == "link"
    return role == expected


def _element_by(
    observation: OrynObservation,
    *,
    action: str,
    role: str | None = None,
    name: str | None = None,
) -> dict[str, Any]:
    candidates = _actionable(observation, action)
    if role is not None:
        candidates = [item for item in candidates if _role_matches(item, role)]
    if name is not None:
        candidates = [item for item in candidates if item.get("text") == name]
    if not candidates:
        raise RuntimeError(f"missing {action} affordance role={role!r} name={name!r}")
    return min(
        candidates,
        key=lambda item: item.get("document_order") or item.get("id") or 0,
    )


def _quoted(command: str, target: str, value: str | None = None) -> str:
    result = f"{command} {json.dumps(target)}"
    return result if value is None else f"{result} {json.dumps(value)}"


class MiniWoBOilOracle:
    """Deterministic controller limited to observations, instructions, and OIL."""

    def __init__(self, oryn: OrynInterface, task: str):
        self.oryn = oryn
        self.task = task
        self.turns: list[OracleTurn] = []
        self.observation: OrynObservation | None = None
        self.trace_cursor = 0
        self.expected_effects = 0
        self.observed_expected_effects = 0
        self.observed_effects = 0

    def observe(self) -> OrynObservation:
        self.observation = self.oryn.observe()
        return self.observation

    def execute(self, oil: str, resolution: dict[str, Any] | None = None) -> OrynResult:
        before = self.observation
        result = self.oryn.execute(oil)
        after = self.observe()
        effects = [_serializable(effect) for effect in result.effects]
        expected = self._expected_effect_kinds(oil)
        observed = {effect.get("kind") for effect in effects}
        self.expected_effects += len(expected)
        self.observed_expected_effects += len(expected & observed)
        self.observed_effects += len(observed)
        reason = None
        if not result.success:
            reason = (
                "capability_unsupported"
                if result.classification == "unsupported"
                else "oil_action_failed"
            )
        self.turns.append(
            OracleTurn(
                observation=_observation_record(before),
                oil=oil,
                resolution=resolution or {},
                result=_serializable(result),
                effects=effects,
                trace_slice=[],
                task_reward=None,
                failure_domain_reason_code=reason,
            )
        )
        if not result.success:
            raise RuntimeError(result.error or f"OIL failed: {oil}")
        return result

    def _expected_effect_kinds(self, oil: str) -> set[str]:
        action = oil.split(maxsplit=1)[0]
        if action in {"click", "type", "clear", "check", "focus", "select", "submit"}:
            return {"event"}
        if action in {"goto", "back", "forward", "refresh"}:
            return {"navigation"}
        return set()

    def solve(self) -> dict[str, Any]:
        initial = self.observe()
        start = _element_by(initial, action="click", name="START")
        self.execute('click "START"', {"alias": start.get("id"), "name": "START"})
        instruction = str(_json_value(self.oryn.execute("text").raw))
        generated = self._instruction(instruction)
        checkpoint_complete = self._has_required_affordances(generated)

        if self.task == "click-button":
            target = re.search(r'Click on the "([^"]+)" button\.', generated).group(1)
            self.execute(_quoted("click", target), {"name": target})
        elif self.task == "click-link":
            target = re.search(r'Click on the link "([^"]+)"\.', generated).group(1)
            self.execute(_quoted("click", target), {"name": target})
        elif self.task == "enter-text":
            value = re.search(r'Enter "([^"]+)" into the text field', generated).group(
                1
            )
            textbox = _element_by(self.observation, action="type", role="textbox")
            self.execute(
                f"type {textbox['id']} {json.dumps(value)}", {"alias": textbox["id"]}
            )
            self.execute('click "Submit"', {"name": "Submit"})
        elif self.task == "focus-text":
            textbox = _element_by(self.observation, action="focus", role="textbox")
            self.execute(f"focus {textbox['id']}", {"alias": textbox["id"]})
        elif self.task == "login-user":
            match = re.search(
                r'username "([^"]+)" and the password "([^"]+)"', generated
            )
            username, password = match.groups()
            textboxes = sorted(
                _actionable(self.observation, "type"),
                key=lambda item: item.get("document_order") or item.get("id") or 0,
            )
            if len(textboxes) < 2:
                raise RuntimeError("login task lacks two writable textboxes")
            self.execute(
                f"type {textboxes[0]['id']} {json.dumps(username)}",
                {"alias": textboxes[0]["id"]},
            )
            self.execute(
                f"type {textboxes[1]['id']} {json.dumps(password)}",
                {"alias": textboxes[1]["id"]},
            )
            self.execute('click "Login"', {"name": "Login"})
        elif self.task == "click-option":
            target = re.search(r"Select (.+?) and click Submit\.", generated).group(1)
            try:
                radio = _element_by(
                    self.observation,
                    action="check",
                    role="radio",
                    name=target,
                )
            except RuntimeError:
                # The compatibility scanner reports the radio's HTML value ("on")
                # while exposing its visible label as the preceding semantic node.
                # Resolve that observable label-to-control relationship without DOM
                # inspection or JavaScript.
                ordered = sorted(
                    self.observation.elements,
                    key=lambda item: item.get("document_order") or item.get("id") or 0,
                )
                label_index = next(
                    (
                        index
                        for index, item in enumerate(ordered)
                        if item.get("text") == target and item.get("type") == "label"
                    ),
                    None,
                )
                if label_index is None:
                    raise
                radio = next(
                    (
                        item
                        for item in ordered[label_index + 1 :]
                        if _role_matches(item, "radio")
                    ),
                    None,
                )
                if radio is None:
                    raise
            self.execute(f"check {radio['id']}", {"alias": radio["id"], "name": target})
            self.execute('click "Submit"', {"name": "Submit"})
        elif self.task == "choose-date":
            date = re.search(r"Select (\d{2})/(\d{2})/(\d{4}) as the date", generated)
            month, day, year = (int(item) for item in date.groups())
            datebox = _element_by(self.observation, action="focus", role="textbox")
            self.execute(f"focus {datebox['id']}", {"alias": datebox["id"]})
            calendar_text = html.unescape(
                str(_json_value(self.oryn.execute("text").raw))
            )
            calendar = re.search(
                rf"({'|'.join(MONTHS)})[^0-9]*(20\d{{2}})", calendar_text
            )
            if not calendar:
                raise RuntimeError("datepicker did not expose its month and year")
            current = int(calendar.group(2)) * 12 + MONTHS.index(calendar.group(1))
            wanted = year * 12 + month - 1
            direction = "Next" if wanted > current else "Prev"
            for _ in range(abs(wanted - current)):
                self.execute(_quoted("click", direction), {"name": direction})
            day_link = _element_by(
                self.observation,
                action="click",
                role="link",
                name=str(day),
            )
            self.execute(
                f"click {day_link['id']}",
                {"alias": day_link["id"], "name": str(day)},
            )
            # The pinned jQuery UI version leaves its popup above the task's
            # submit button after programmatic activation. The selected value is
            # observable on the readonly input, so use OIL's explicit force flag
            # to submit the now-complete form in both execution domains.
            self.execute('click "Submit" --force', {"name": "Submit"})
        elif self.task == "search-engine":
            match = re.search(
                r'enter "([^"]+)".*click the (\d+)(?:st|nd|rd|th) search result',
                generated,
                re.IGNORECASE,
            )
            query, rank_text = match.groups()
            rank = int(rank_text)
            textbox = _element_by(self.observation, action="type", role="textbox")
            self.execute(
                f"type {textbox['id']} {json.dumps(query)}", {"alias": textbox["id"]}
            )
            self.execute('click "Search"', {"name": "Search"})
            page = math.ceil(rank / 3)
            if page > 1:
                self.execute(_quoted("click", str(page)), {"name": str(page)})
            excluded = {"", "First", "Last", "<", ">", "1", "2", "3"}
            results = sorted(
                [
                    item
                    for item in _actionable(self.observation, "click")
                    if _role_matches(item, "link")
                    and item.get("text")
                    and item.get("text") not in excluded
                ],
                key=lambda item: item.get("document_order") or item.get("id") or 0,
            )
            offset = (rank - 1) % 3
            if len(results) <= offset:
                raise RuntimeError(f"search result rank {rank} is not observable")
            target = results[offset]
            self.execute(
                f"click {target['id']}",
                {"alias": target["id"], "name": target.get("text")},
            )
        else:
            raise RuntimeError(f"unsupported oracle task: {self.task}")

        text = str(_json_value(self.oryn.execute("text").raw))
        trace_result = self.oryn.execute("trace stop")
        trace = _json_trace(trace_result.raw)
        reward = self._reward(text, trace)
        episode_done = "Last reward: -" not in text or reward is not None
        success = reward is not None and reward > 0
        if self.turns:
            self.turns[-1].trace_slice = trace
            self.turns[-1].task_reward = reward
        unexpected = self._unexpected_capabilities()
        return {
            "task_id": self.task,
            "success": success,
            "episode_done": episode_done,
            "reward": reward,
            "instruction": generated,
            "observation_sufficiency": {
                "complete_checkpoints": int(checkpoint_complete),
                "required_checkpoints": 1,
                "rate": 1.0 if checkpoint_complete else 0.0,
            },
            "causal": {
                "expected_effects": self.expected_effects,
                "observed_expected_effects": self.observed_expected_effects,
                "observed_effects": self.observed_effects,
                "recall": (
                    self.observed_expected_effects / self.expected_effects
                    if self.expected_effects
                    else None
                ),
                "precision": (
                    self.observed_expected_effects / self.observed_effects
                    if self.observed_effects
                    else None
                ),
            },
            "unexpected_capabilities": unexpected,
            "turns": [asdict(turn) for turn in self.turns],
        }

    def _instruction(self, text: str) -> str:
        marker = text.find("Last reward:")
        content = text[:marker].strip() if marker >= 0 else text.strip()
        patterns = {
            "click-button": r'Click on the "[^"]+" button\.',
            "click-link": r'Click on the link "[^"]+"\.',
            "click-option": r"Select .+? and click Submit\.",
            "enter-text": r'Enter "[^"]+" into the text field and press Submit\.',
            "focus-text": r"Focus into the textbox\.",
            "choose-date": r"Select \d{2}/\d{2}/\d{4} as the date and hit submit\.",
            "login-user": (
                r'Enter the username "[^"]+" and the password "[^"]+" '
                r"into the text fields and press login\."
            ),
            "search-engine": (
                r'Use the textbox to enter "[^"]+" and press "Search", then find '
                r"and click the \d+(?:st|nd|rd|th) search result\."
            ),
        }
        match = re.search(patterns[self.task], content, re.DOTALL)
        if not match:
            raise RuntimeError(f"task instruction was not observable for {self.task}")
        return " ".join(match.group(0).split())

    def _has_required_affordances(self, instruction: str) -> bool:
        if not instruction:
            return False
        required = {
            "click-button": [("click", "button")],
            "click-link": [("click", None)],
            "click-option": [("check", "radio"), ("click", "button")],
            "enter-text": [("type", "textbox"), ("click", "button")],
            "focus-text": [("focus", "textbox")],
            "choose-date": [("focus", "textbox"), ("click", "button")],
            "login-user": [("type", "textbox"), ("click", "button")],
            "search-engine": [("type", "textbox"), ("click", "button")],
        }[self.task]
        return all(
            any(
                role is None or _role_matches(item, role)
                for item in _actionable(self.observation, action)
            )
            for action, role in required
        )

    def _unexpected_capabilities(self) -> list[dict[str, Any]]:
        if not self.observation:
            return []
        return [
            _serializable(item)
            for item in self.observation.capabilities
            if getattr(item, "support", None) != "supported"
            and getattr(item, "capability", None) not in ALLOWED_NATIVE_LIMITATIONS
        ]

    @staticmethod
    def _reward(text: str, trace: list[dict[str, Any]]) -> float | None:
        for event in reversed(trace):
            kind = event.get("kind") or {}
            if kind.get("kind") != "console":
                continue
            match = re.search(
                r"\(raw:\s*(-?\d+(?:\.\d+)?)\)", str(kind.get("message", ""))
            )
            if match:
                return float(match.group(1))
        match = re.search(r"Last reward:\s*(-?\d+(?:\.\d+)?)", text)
        return float(match.group(1)) if match else None


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def _first_divergence(
    native: dict[str, Any], chromium: dict[str, Any]
) -> dict[str, Any] | None:
    if native.get("instruction") != chromium.get("instruction"):
        return {
            "checkpoint": "instruction",
            "native": native.get("instruction"),
            "chromium": chromium.get("instruction"),
        }
    native_turns = native.get("turns") or []
    chromium_turns = chromium.get("turns") or []
    for index in range(max(len(native_turns), len(chromium_turns))):
        if index >= len(native_turns) or index >= len(chromium_turns):
            return {
                "checkpoint": f"turn_{index + 1}_count",
                "native": len(native_turns),
                "chromium": len(chromium_turns),
            }
        native_turn = native_turns[index]
        chromium_turn = chromium_turns[index]
        native_resolution = {
            key: value
            for key, value in (native_turn.get("resolution") or {}).items()
            if key != "alias"
        }
        chromium_resolution = {
            key: value
            for key, value in (chromium_turn.get("resolution") or {}).items()
            if key != "alias"
        }
        if native_resolution != chromium_resolution:
            return {
                "checkpoint": f"turn_{index + 1}_resolution",
                "native": native_resolution,
                "chromium": chromium_resolution,
            }
    if native.get("success") != chromium.get("success"):
        return {
            "checkpoint": "outcome",
            "native": native.get("success"),
            "chromium": chromium.get("success"),
        }
    return None


def run_cell(
    args: argparse.Namespace,
    task: str,
    seed: int,
    domain: str,
    fingerprint_sha256: str,
) -> dict[str, Any]:
    options: dict[str, Any] = {
        "binary_path": str(args.oryn_binary),
        "timeout": args.command_timeout,
    }
    mode = "native" if domain == "native" else "headless"
    if domain == "native":
        options["cli_args"] = ["--allow-loopback"]
    else:
        options["env"] = {"CHROME_BIN": str(args.chrome_binary)}
    url = f"{args.miniwob_url.rstrip('/')}/miniwob/{task}.html?oryn_seed={seed}"
    started = time.perf_counter()
    with OrynInterface(mode=mode, **options) as oryn:
        goto = oryn.goto(url)
        if not goto.success:
            raise RuntimeError(goto.error or f"navigation failed: {url}")
        oracle = MiniWoBOilOracle(oryn, task)
        try:
            result = oracle.solve()
        except Exception as error:
            result = {
                "task_id": task,
                "success": False,
                "failure_domain_reason_code": "oracle_or_runtime_failure",
                "error": f"{type(error).__name__}: {error}",
                "turns": [asdict(turn) for turn in oracle.turns],
            }
    return {
        "cell_id": f"{task}-s{seed}-{domain}-oracle",
        "fingerprint_sha256": fingerprint_sha256,
        "seed": seed,
        "domain": domain,
        "duration_ms": (time.perf_counter() - started) * 1000,
        **result,
    }


def main() -> int:
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=repo_root)
    parser.add_argument(
        "--oryn-binary", type=Path, default=repo_root / "target/release/oryn"
    )
    parser.add_argument(
        "--chrome-binary",
        type=Path,
        default=Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
    )
    parser.add_argument("--miniwob-url", default="http://127.0.0.1:8765")
    parser.add_argument(
        "--domain", choices=("native", "chromium", "both"), default="native"
    )
    parser.add_argument("--task", choices=TASKS, action="append")
    parser.add_argument("--seed", choices=SEEDS, type=int, action="append")
    parser.add_argument("--command-timeout", type=float, default=120.0)
    parser.add_argument(
        "--output", type=Path, default=repo_root / "artifacts/g2r/deterministic"
    )
    args = parser.parse_args()

    domains = ("native", "chromium") if args.domain == "both" else (args.domain,)
    fingerprint = _run_fingerprint(args, domains)
    started_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    cells = []
    selected_tasks = tuple(args.task or TASKS)
    selected_seeds = tuple(args.seed or SEEDS)
    expected_cells_per_domain = len(selected_tasks) * len(selected_seeds)
    for domain in domains:
        for task in selected_tasks:
            for seed in selected_seeds:
                try:
                    cell = run_cell(
                        args,
                        task,
                        seed,
                        domain,
                        fingerprint["sha256"],
                    )
                except Exception as error:
                    cell = {
                        "cell_id": f"{task}-s{seed}-{domain}-oracle",
                        "fingerprint_sha256": fingerprint["sha256"],
                        "task_id": task,
                        "seed": seed,
                        "domain": domain,
                        "success": False,
                        "failure_domain_reason_code": "oracle_or_runtime_failure",
                        "error": f"{type(error).__name__}: {error}",
                    }
                _write_json(args.output / f"{cell['cell_id']}.json", cell)
                cells.append(cell)
                print(
                    json.dumps(
                        {
                            "cell_id": cell["cell_id"],
                            "success": cell.get("success", False),
                            "error": cell.get("error"),
                        }
                    ),
                    flush=True,
                )
    native = [cell for cell in cells if cell["domain"] == "native"]
    chromium = [cell for cell in cells if cell["domain"] == "chromium"]
    differential = []
    if (
        selected_tasks == TASKS
        and selected_seeds == SEEDS
        and len(native) == 24
        and len(chromium) == 24
    ):
        by_id = {
            (cell["task_id"], cell["seed"], cell["domain"]): cell for cell in cells
        }
        for task in TASKS:
            for seed in SEEDS:
                divergence = _first_divergence(
                    by_id[(task, seed, "native")], by_id[(task, seed, "chromium")]
                )
                differential.append(
                    {
                        "task_id": task,
                        "seed": seed,
                        "first_divergence": divergence,
                    }
                )
    requested_complete = all(
        len([cell for cell in cells if cell["domain"] == domain])
        == expected_cells_per_domain
        and all(cell.get("success") for cell in cells if cell["domain"] == domain)
        for domain in domains
    )
    parity_complete = not differential or all(
        item["first_divergence"] is None for item in differential
    )
    aggregate = {
        "schema_version": 3,
        "run_id": "g2r-deterministic-v1",
        "fingerprint": fingerprint,
        "started_at": started_at,
        "completed_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "cells": len(cells),
        "native": {
            "completed": len(native),
            "passed": sum(bool(cell.get("success")) for cell in native),
        },
        "chromium": {
            "completed": len(chromium),
            "passed": sum(bool(cell.get("success")) for cell in chromium),
        },
        "differential": differential,
        "outcome": {
            "status": "passed" if requested_complete and parity_complete else "failed"
        },
    }
    _write_json(args.output / "aggregate.json", aggregate)
    return 0 if aggregate["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
