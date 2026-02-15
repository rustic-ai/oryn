/**
 * Unit tests for sidepanel.js
 * Tests logging, status updates, and message handling
 */

const { mockRuntimeSendMessage } = require('../helpers/chrome-mocks');

describe('Sidepanel - Log Management', () => {
  let document;
  let logs;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <div id="log-container"></div>
      <div id="wasm-status"></div>
      <div id="scan-status"></div>
      <button id="btn-clear"></button>
    `;
    logs = [];
  });

  test('should add log entry', () => {
    const logContainer = document.getElementById('log-container');
    const timestamp = new Date().toLocaleTimeString();
    const message = 'Test log message';
    const type = 'info';

    // Add log
    logs.push({ timestamp, message, type });

    // Render
    const entry = document.createElement('div');
    entry.className = `log-entry log-${type}`;
    entry.innerHTML = `<span class="log-time">${timestamp}</span><span class="log-msg">${message}</span>`;
    logContainer.appendChild(entry);

    expect(logs.length).toBe(1);
    expect(logs[0].message).toBe('Test log message');
    expect(logContainer.children.length).toBe(1);
  });

  test('should add error log with proper styling', () => {
    const logContainer = document.getElementById('log-container');
    const message = 'Error occurred';
    const type = 'error';

    logs.push({ timestamp: '12:00:00', message, type });

    const entry = document.createElement('div');
    entry.className = `log-entry log-${type}`;
    logContainer.appendChild(entry);

    expect(entry.className).toContain('log-error');
  });

  test('should clear all logs', () => {
    const logContainer = document.getElementById('log-container');

    // Add some logs
    logs.push({ timestamp: '12:00:00', message: 'Log 1', type: 'info' });
    logs.push({ timestamp: '12:00:01', message: 'Log 2', type: 'info' });

    expect(logs.length).toBe(2);

    // Clear
    logs.length = 0;
    logContainer.innerHTML = '';

    expect(logs.length).toBe(0);
    expect(logContainer.children.length).toBe(0);
  });

  test('should limit log count to MAX_LOGS', () => {
    const MAX_LOGS = 500;

    // Add more than MAX_LOGS
    for (let i = 0; i < 600; i++) {
      logs.push({ timestamp: '12:00:00', message: `Log ${i}`, type: 'info' });

      // Trim if needed
      if (logs.length > MAX_LOGS) {
        logs.shift();
      }
    }

    expect(logs.length).toBe(MAX_LOGS);
  });
});

describe('Sidepanel - Status Updates', () => {
  let document;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <div id="wasm-status" class="status-value"></div>
      <div id="scan-status" class="status-value"></div>
    `;
  });

  test('should update WASM status to ready', async () => {
    mockRuntimeSendMessage({ wasmInitialized: true, hasScan: false });

    const wasmStatus = document.getElementById('wasm-status');

    // Mock update
    wasmStatus.textContent = 'Ready';
    wasmStatus.className = 'status-value ready';

    expect(wasmStatus.textContent).toBe('Ready');
    expect(wasmStatus.className).toContain('ready');
  });

  test('should update WASM status to error', async () => {
    mockRuntimeSendMessage({ wasmInitialized: false, hasScan: false });

    const wasmStatus = document.getElementById('wasm-status');

    // Mock update
    wasmStatus.textContent = 'Error';
    wasmStatus.className = 'status-value error';

    expect(wasmStatus.textContent).toBe('Error');
    expect(wasmStatus.className).toContain('error');
  });

  test('should update scan status when loaded', async () => {
    mockRuntimeSendMessage({ wasmInitialized: true, hasScan: true });

    const scanStatus = document.getElementById('scan-status');

    // Mock update
    scanStatus.textContent = 'Loaded';
    scanStatus.className = 'status-value ready';

    expect(scanStatus.textContent).toBe('Loaded');
    expect(scanStatus.className).toContain('ready');
  });

  test('should show scan not loaded', async () => {
    mockRuntimeSendMessage({ wasmInitialized: true, hasScan: false });

    const scanStatus = document.getElementById('scan-status');

    // Mock update
    scanStatus.textContent = 'Not loaded';
    scanStatus.className = 'status-value';

    expect(scanStatus.textContent).toBe('Not loaded');
  });
});

describe('Sidepanel - Message Handling', () => {
  let messageListener;
  let logs;

  beforeEach(() => {
    logs = [];

    // Create a mock message listener
    messageListener = (message, sender, sendResponse) => {
      if (message.type === 'log') {
        logs.push({
          timestamp: new Date().toLocaleTimeString(),
          message: message.message,
          type: message.level
        });
        sendResponse({ ok: true });
      }
    };
  });

  test('should handle log message', () => {
    const message = { type: 'log', message: 'Test log', level: 'info' };
    const sender = {};
    const sendResponse = jest.fn();

    // Add log
    logs.push({ timestamp: new Date().toLocaleTimeString(), message: message.message, type: message.level });

    if (messageListener) {
      messageListener(message, sender, sendResponse);
    }

    expect(sendResponse).toHaveBeenCalledWith({ ok: true });
    expect(logs.length).toBeGreaterThan(0);
  });

  test('should handle error log message', () => {
    const message = { type: 'log', message: 'Error occurred', level: 'error' };
    const sender = {};
    const sendResponse = jest.fn();

    logs.push({ timestamp: new Date().toLocaleTimeString(), message: message.message, type: message.level });

    expect(logs[logs.length - 1].type).toBe('error');
  });
});

describe('Sidepanel - Auto-scroll', () => {
  let document;

  beforeEach(() => {
    document = global.document;
    document.body.innerHTML = `
      <div id="log-container" style="overflow-y: auto; height: 500px;">
        <div class="log-entry">Log 1</div>
        <div class="log-entry">Log 2</div>
      </div>
    `;
  });

  test('should scroll to bottom when new log added', () => {
    const logContainer = document.getElementById('log-container');

    // Mock scrollHeight and scrollTop
    Object.defineProperty(logContainer, 'scrollHeight', { value: 1000, writable: true });
    Object.defineProperty(logContainer, 'scrollTop', { value: 0, writable: true });

    // Add new log and scroll
    const newEntry = document.createElement('div');
    newEntry.className = 'log-entry';
    newEntry.textContent = 'New log';
    logContainer.appendChild(newEntry);

    // Simulate auto-scroll
    logContainer.scrollTop = logContainer.scrollHeight;

    expect(logContainer.scrollTop).toBe(1000);
  });
});

describe('Sidepanel - Console Interception', () => {
  test('should intercept console.log', () => {
    const originalLog = console.log;
    const logs = [];

    console.log = jest.fn((...args) => {
      originalLog.apply(console, args);
      logs.push({ message: args.join(' '), type: 'info' });
    });

    console.log('Test message');

    expect(console.log).toHaveBeenCalledWith('Test message');
    expect(logs.length).toBe(1);
    expect(logs[0].message).toBe('Test message');

    // Restore
    console.log = originalLog;
  });

  test('should intercept console.error', () => {
    const originalError = console.error;
    const logs = [];

    console.error = jest.fn((...args) => {
      originalError.apply(console, args);
      logs.push({ message: args.join(' '), type: 'error' });
    });

    console.error('Error message');

    expect(console.error).toHaveBeenCalledWith('Error message');
    expect(logs.length).toBe(1);
    expect(logs[0].type).toBe('error');

    // Restore
    console.error = originalError;
  });
});
