from intentgym.core.oryn import OrynObservation
from scripts.run_g2_deterministic import (
    _first_divergence,
    _is_supported_native_runtime,
    _observation_text,
)
from scripts.serve_g2_miniwob import inject_seed


def test_seed_injection_is_deterministic_and_after_core():
    source = b'<script src="../core/core.js"></script><script>genProblem()</script>'

    first = inject_seed(source, 17)
    repeated = inject_seed(source, 17)
    different = inject_seed(source, 42)

    assert first == repeated
    assert first != different
    assert first.index(b"Math.seedrandom") > first.index(b"../core/core.js")
    assert first.index(b"Math.seedrandom") < first.index(b"genProblem")


def test_differential_reports_first_instruction_divergence():
    native = {"instruction": "Click A", "turns": [], "success": True}
    chromium = {"instruction": "Click B", "turns": [], "success": True}

    assert _first_divergence(native, chromium) == {
        "checkpoint": "instruction",
        "native": "Click A",
        "chromium": "Click B",
    }


def test_secure_native_runtime_accepts_v3_and_v4_protocols():
    runtime = {
        "native_v8": True,
        "build_profile": "release",
        "execution_mode": "sandboxed_worker",
        "sandboxed": True,
        "worker_protocol_version": 3,
    }

    assert _is_supported_native_runtime(runtime)
    assert _is_supported_native_runtime({**runtime, "worker_protocol_version": 4})
    assert not _is_supported_native_runtime({**runtime, "worker_protocol_version": 2})
    assert not _is_supported_native_runtime({**runtime, "sandboxed": False})


def test_observation_text_exposes_dynamic_datepicker_heading():
    observation = OrynObservation(
        raw="",
        url="https://example.test",
        title="Date",
        elements=[
            {"role": "button", "text": "Submit"},
            {"role": "body", "text": "PrevNextDecember&#xa0;2016SuMoTuWeThFrSa"},
        ],
    )

    assert "December\N{NO-BREAK SPACE}2016" in _observation_text(observation)
