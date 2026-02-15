"""Tests for BenchmarkRunner recovery behavior."""

from types import SimpleNamespace

from intentgym.benchmarks.base import Task
from intentgym.collection.metrics import EpisodeMetrics
from intentgym.core.oryn import OrynObservation, OrynResult
from intentgym.core.runner import BenchmarkRunner


class _FakeAgent:
    def reset(self):
        return None


class _FakeOryn:
    def __init__(self, fail_first_observe: bool = False):
        self.fail_first_observe = fail_first_observe
        self.observe_calls = 0
        self.goto_calls = []

    def goto(self, url: str):
        self.goto_calls.append(url)
        return OrynResult(success=True, raw=f"goto {url}")

    def observe(self):
        self.observe_calls += 1
        if self.fail_first_observe and self.observe_calls == 1:
            raise RuntimeError("Operation 'scan' timed out after 60s")
        return OrynObservation(
            raw='timer: 10 / 10 sec remaining',
            url="http://localhost:8765/miniwob/click-button.html",
            title="Click Button Task",
        )

    def execute(self, command: str):
        return OrynResult(success=True, raw=f"ok {command}")

    def close(self):
        return None


def _make_episode(episode_number: int, success: bool, error: str | None = None):
    return EpisodeMetrics(
        episode_number=episode_number,
        success=success,
        partial_score=1.0 if success else 0.0,
        total_steps=2 if success else 0,
        total_input_tokens=10,
        total_output_tokens=5,
        total_observation_tokens=4,
        total_cost_usd=0.001,
        total_duration_ms=100.0,
        observation_ratio=0.4,
        peak_context_tokens=20,
        failed_actions=0 if success else 1,
        timeout=False,
        error=error,
        turns=[],
    )


def test_is_recoverable_error_detection():
    runner = BenchmarkRunner.__new__(BenchmarkRunner)

    assert runner._is_recoverable_error("Operation 'scan' timed out after 60s")
    assert runner._is_recoverable_error("ConnectionLostError: subprocess exited")
    assert not runner._is_recoverable_error("ValidationError: bad prompt")


def test_multi_episode_prestart_timeout_marks_episode_failed_and_continues():
    runner = BenchmarkRunner.__new__(BenchmarkRunner)
    runner.config = SimpleNamespace(
        save_transcript=False,
        run_id="test-run",
        oryn_mode="headless",
        oryn_options={},
    )
    runner.agent = _FakeAgent()
    runner.oryn = _FakeOryn(fail_first_observe=True)

    restart_reasons = []

    def fake_restart(reason: str, attempts: int = 1):
        restart_reasons.append(reason)
        runner.oryn = _FakeOryn(fail_first_observe=False)
        return True

    runner._restart_oryn_session = fake_restart

    def fake_run_single_episode(task, episode_num, transcript=None):
        return _make_episode(episode_num, success=True)

    runner._run_single_episode = fake_run_single_episode

    task = Task(
        task_id="click-button",
        intent="Click the requested button",
        start_url="http://localhost:8765/miniwob/click-button.html",
        success_criteria={},
    )
    result = runner._run_task_multi_episode(task, num_episodes=2)

    assert len(restart_reasons) >= 1
    assert result.is_multi_episode
    assert result.episodes_count == 2
    assert result.episodes_succeeded == 1
    assert result.timeout_count == 1
    assert result.success_rate == 0.5
