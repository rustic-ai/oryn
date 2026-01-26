"""E2E tests that run actual .oil scripts from test-harness/scripts/.

These tests execute the same .oil scripts used by the Rust E2E tests,
verifying that the Python client can run them successfully.
"""


import pytest
from oryn import run_oil_file_sync

# List of .oil scripts to test
OIL_SCRIPTS = [
    "01_static.oil",
    "02_forms.oil",
    "03_ecommerce.oil",
    "04_interactivity.oil",
    "05_dynamic.oil",
    "06_edge_cases.oil",
    "07_intents_builtin.oil",
    "08_multipage_flows.oil",
    "09_target_resolution.oil",
]


def is_success(response: str) -> bool:
    """Check if a response string indicates success."""
    lower = response.lower()
    if lower.startswith("error:"):
        return False
    if "unknown command:" in lower:
        return False
    if "element not found" in lower:
        return False
    if "navigation failed" in lower:
        return False
    return True


@pytest.mark.e2e
@pytest.mark.oil
class TestOilScripts:
    """Run .oil scripts against the test harness."""

    @pytest.mark.parametrize("script_name", OIL_SCRIPTS)
    def test_oil_script(self, client, scripts_dir, script_name):
        """Run a single .oil script and verify all commands succeed."""
        script_path = scripts_dir / script_name

        if not script_path.exists():
            pytest.skip(f"Script not found: {script_path}")

        # Run the script
        results = run_oil_file_sync(client, script_path)

        # Check results
        failures = []
        for cmd, result in results:
            if not is_success(result):
                failures.append(f"  {cmd}: {result}")

        if failures:
            failure_msg = "\n".join(failures)
            pytest.fail(f"Script {script_name} had failures:\n{failure_msg}")

        # Report success
        print(f"\n{script_name}: {len(results)} commands executed successfully")


@pytest.mark.e2e
@pytest.mark.oil
class TestOilScriptDetails:
    """Detailed tests for specific .oil scripts with assertions."""

    def test_01_static(self, client, scripts_dir):
        """Test static page navigation and extraction."""
        script_path = scripts_dir / "01_static.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)

        # Should have executed commands
        assert len(results) > 0

        # Check for navigation success
        nav_results = [r for cmd, r in results if cmd.startswith("goto")]
        for result in nav_results:
            assert is_success(result), f"Navigation failed: {result}"

    def test_02_forms(self, client, scripts_dir):
        """Test form interactions."""
        script_path = scripts_dir / "02_forms.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)

        assert len(results) > 0

        # Check type commands succeeded
        type_results = [r for cmd, r in results if cmd.startswith("type")]
        for result in type_results:
            assert is_success(result), f"Type command failed: {result}"

    def test_03_ecommerce(self, client, scripts_dir):
        """Test e-commerce flow."""
        script_path = scripts_dir / "03_ecommerce.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_04_interactivity(self, client, scripts_dir):
        """Test interactive elements."""
        script_path = scripts_dir / "04_interactivity.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_05_dynamic(self, client, scripts_dir):
        """Test dynamic content handling."""
        script_path = scripts_dir / "05_dynamic.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_06_edge_cases(self, client, scripts_dir):
        """Test edge case handling."""
        script_path = scripts_dir / "06_edge_cases.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_07_intents_builtin(self, client, scripts_dir):
        """Test built-in intents."""
        script_path = scripts_dir / "07_intents_builtin.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_08_multipage_flows(self, client, scripts_dir):
        """Test multi-page navigation flows."""
        script_path = scripts_dir / "08_multipage_flows.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0

    def test_09_target_resolution(self, client, scripts_dir):
        """Test target resolution."""
        script_path = scripts_dir / "09_target_resolution.oil"
        if not script_path.exists():
            pytest.skip("Script not found")

        results = run_oil_file_sync(client, script_path)
        assert len(results) > 0


@pytest.mark.e2e
@pytest.mark.oil
def test_run_all_scripts_sequentially(client, scripts_dir):
    """Run all .oil scripts in sequence."""
    total_commands = 0
    total_failures = 0

    for script_name in OIL_SCRIPTS:
        script_path = scripts_dir / script_name
        if not script_path.exists():
            print(f"SKIP: {script_name} (not found)")
            continue

        results = run_oil_file_sync(client, script_path)
        failures = sum(1 for _, r in results if not is_success(r))

        total_commands += len(results)
        total_failures += failures

        status = "PASS" if failures == 0 else f"FAIL ({failures} errors)"
        print(f"{status}: {script_name} ({len(results)} commands)")

    print(f"\nTotal: {total_commands} commands, {total_failures} failures")
    assert total_failures == 0, f"{total_failures} commands failed across all scripts"
