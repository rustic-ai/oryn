from scripts.run_g2_deterministic import _first_divergence
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
