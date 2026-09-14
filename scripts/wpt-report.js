/* Versioned reporting adapter: no HTML rewriting or DOM result element. */
(() => {
  // Establish native reference focus after document initialization, before the
  // test scripts that follow this adapter. A document-creation hook runs too
  // early to establish this prerequisite. No focus emulation is used.
  if (typeof __oryn_reference_focus === 'function' && window === top) {
    window.focus();
    __oryn_reference_focus(JSON.stringify({phase:'report_ready', focused:document.hasFocus()}));
  }
  // Use testharness's standard machine-runner mode. Its visual results table
  // is not test content and can dominate completion time on large suites.
  setup({output: false});
  let completions = 0;
  globalThis.__oryn_wpt_completion = null;
  add_completion_callback((tests, harness) => {
    globalThis.__oryn_wpt_completion = {
      complete: true,
      completion_count: ++completions,
      harness: harness.status,
      harnessMessage: String(harness.message || ''),
      tests: tests.map(test => ({name: String(test.name), status: test.status,
                                message: String(test.message || '')})),
      testdriver: globalThis.__oryn_testdriver_log || [],
      automation_overflow: globalThis.__oryn_testdriver_overflow === true
    };
  });
})();
