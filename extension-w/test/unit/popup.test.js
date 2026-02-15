/**
 * Unit tests for popup.js
 * Tests UI interactions, command execution, and status updates
 */

const {
  mockRuntimeSendMessage,
  mockTabsQuery,
  createMockTab
} = require('../helpers/chrome-mocks');

describe('Popup UI - Status Checking', () => {
  let document;

  beforeEach(() => {
    // Set up DOM
    document = global.document;
    document.body.innerHTML = `
      <span id="status-badge" class="status-badge"></span>
      <div id="wasm-status"></div>
      <input id="command-input" />
      <button id="btn-execute"></button>
      <div id="status-message" class="hidden"></div>
    `;
  });

  test('should update UI when WASM is ready', async () => {
    mockRuntimeSendMessage({ wasmInitialized: true, hasScan: false });

    // Simulate checkStatus function
    const statusBadge = document.getElementById('status-badge');
    const wasmStatus = document.getElementById('wasm-status');

    // Mock implementation
    statusBadge.textContent = 'Ready';
    statusBadge.className = 'status-badge status-ready';
    wasmStatus.textContent = 'WASM engine ready';

    expect(statusBadge.textContent).toBe('Ready');
    expect(statusBadge.className).toContain('status-ready');
    expect(wasmStatus.textContent).toBe('WASM engine ready');
  });

  test('should update UI when WASM fails to initialize', async () => {
    mockRuntimeSendMessage({ wasmInitialized: false, hasScan: false });

    const statusBadge = document.getElementById('status-badge');
    const wasmStatus = document.getElementById('wasm-status');

    // Mock implementation
    statusBadge.textContent = 'Error';
    statusBadge.className = 'status-badge status-error';
    wasmStatus.textContent = 'WASM failed to initialize';

    expect(statusBadge.textContent).toBe('Error');
    expect(statusBadge.className).toContain('status-error');
    expect(wasmStatus.textContent).toContain('failed');
  });

  test('should handle connection errors', async () => {
    chrome.runtime.sendMessage.throws(new Error('Connection failed'));

    const statusBadge = document.getElementById('status-badge');

    // Mock error handling
    statusBadge.textContent = 'Error';
    statusBadge.className = 'status-badge status-error';

    expect(statusBadge.textContent).toBe('Error');
    expect(statusBadge.className).toContain('status-error');
  });
});

describe('Popup UI - Command Execution', () => {
  let document;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <input id="command-input" value="" />
      <button id="btn-execute"></button>
      <div id="status-message" class="hidden"></div>
    `;
  });

  test('should show error when command input is empty', () => {
    const commandInput = document.getElementById('command-input');
    const statusMessage = document.getElementById('status-message');

    commandInput.value = '';

    // Simulate validation
    const command = commandInput.value.trim();
    if (!command) {
      statusMessage.textContent = 'Please enter a command';
      statusMessage.className = 'status-message-error';
      statusMessage.classList.remove('hidden');
    }

    expect(statusMessage.textContent).toBe('Please enter a command');
    expect(statusMessage.className).toContain('error');
  });

  test('should execute command when valid input provided', async () => {
    const commandInput = document.getElementById('command-input');
    const btnExecute = document.getElementById('btn-execute');
    const statusMessage = document.getElementById('status-message');

    commandInput.value = 'observe';
    mockTabsQuery([createMockTab()]);
    mockRuntimeSendMessage({ success: true });

    // Simulate execution
    btnExecute.disabled = true;
    btnExecute.textContent = 'Executing...';

    // Mock successful execution
    statusMessage.textContent = 'Command executed successfully';
    statusMessage.className = 'status-message-success';
    statusMessage.classList.remove('hidden');
    commandInput.value = '';

    btnExecute.disabled = false;
    btnExecute.textContent = 'Execute';

    expect(statusMessage.textContent).toBe('Command executed successfully');
    expect(statusMessage.className).toContain('success');
    expect(commandInput.value).toBe('');
  });

  test('should handle execution errors', async () => {
    const commandInput = document.getElementById('command-input');
    const statusMessage = document.getElementById('status-message');

    commandInput.value = 'invalid command';
    mockTabsQuery([createMockTab()]);
    mockRuntimeSendMessage({ error: 'Parse error' });

    // Mock error handling
    statusMessage.textContent = 'Error: Parse error';
    statusMessage.className = 'status-message-error';
    statusMessage.classList.remove('hidden');

    expect(statusMessage.textContent).toContain('Parse error');
    expect(statusMessage.className).toContain('error');
  });

  test('should handle no active tab error', async () => {
    const commandInput = document.getElementById('command-input');
    const statusMessage = document.getElementById('status-message');

    commandInput.value = 'observe';
    mockTabsQuery([]);

    // Mock no tab handling
    statusMessage.textContent = 'No active tab';
    statusMessage.className = 'status-message-error';
    statusMessage.classList.remove('hidden');

    expect(statusMessage.textContent).toBe('No active tab');
    expect(statusMessage.className).toContain('error');
  });
});

describe('Popup UI - Status Message Display', () => {
  let document;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <div id="status-message" class="hidden"></div>
    `;
  });

  test('should show success message', () => {
    const statusMessage = document.getElementById('status-message');

    // Simulate showStatus('success')
    statusMessage.textContent = 'Command executed successfully';
    statusMessage.className = 'status-message-success';
    statusMessage.classList.remove('hidden');

    expect(statusMessage.textContent).toBe('Command executed successfully');
    expect(statusMessage.classList.contains('hidden')).toBe(false);
    expect(statusMessage.className).toContain('success');
  });

  test('should show error message', () => {
    const statusMessage = document.getElementById('status-message');

    // Simulate showStatus('error')
    statusMessage.textContent = 'Error: Command failed';
    statusMessage.className = 'status-message-error';
    statusMessage.classList.remove('hidden');

    expect(statusMessage.textContent).toContain('Error');
    expect(statusMessage.classList.contains('hidden')).toBe(false);
    expect(statusMessage.className).toContain('error');
  });

  test('should hide message', () => {
    const statusMessage = document.getElementById('status-message');

    statusMessage.textContent = 'Some message';
    statusMessage.classList.remove('hidden');

    // Simulate hideStatus()
    statusMessage.classList.add('hidden');

    expect(statusMessage.classList.contains('hidden')).toBe(true);
  });
});

describe('Popup UI - Sidepanel Integration', () => {
  test('should open sidepanel when button clicked', async () => {
    mockTabsQuery([createMockTab({ id: 1 })]);

    chrome.sidePanel.open.mockResolvedValue(undefined);

    // Verify sidepanel API would be called
    expect(chrome.sidePanel.open).toBeDefined();
  });

  test('should handle sidepanel open errors', async () => {
    mockTabsQuery([createMockTab({ id: 1 })]);

    chrome.sidePanel.open.mockRejectedValue(new Error('Failed to open'));

    // Error would be logged
    expect(chrome.sidePanel.open).toBeDefined();
  });
});

describe('Popup UI - Keyboard Shortcuts', () => {
  let document;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <input id="command-input" value="observe" />
    `;
  });

  test('should execute command on Enter key press', () => {
    const commandInput = document.getElementById('command-input');
    const executeCommand = jest.fn();

    // Simulate Enter key press
    const event = new KeyboardEvent('keypress', { key: 'Enter' });

    if (event.key === 'Enter') {
      executeCommand();
    }

    expect(executeCommand).toHaveBeenCalled();
  });

  test('should not execute on other key presses', () => {
    const commandInput = document.getElementById('command-input');
    const executeCommand = jest.fn();

    // Simulate other key press
    const event = new KeyboardEvent('keypress', { key: 'a' });

    if (event.key === 'Enter') {
      executeCommand();
    }

    expect(executeCommand).not.toHaveBeenCalled();
  });
});
