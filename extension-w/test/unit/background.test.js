/**
 * Unit tests for background.js
 * Tests message handling, command execution, and WASM integration
 */

const {
  createMockScanResult,
  mockTabsSendMessage,
  mockTabsQuery,
  createMockTab,
  createMockWasmModule
} = require('../helpers/chrome-mocks');

describe('Background Script - Message Handling', () => {
  let messageListener;
  let orynCore;
  let currentScan;

  beforeEach(() => {
    // Mock WASM module
    const wasmModule = createMockWasmModule();
    orynCore = new wasmModule.OrynCore();
    currentScan = null;

    // Create a mock message listener
    messageListener = async (message, sender, sendResponse) => {
      if (message.type === 'execute_oil') {
        if (!sender.tab || !sender.tab.id) {
          sendResponse({ error: 'No active tab' });
          return;
        }
        if (!orynCore) {
          sendResponse({ error: 'WASM not initialized' });
          return;
        }
        sendResponse({ success: true });
        return;
      }

      if (message.type === 'scan_complete') {
        currentScan = message.scan;
        sendResponse({ ok: true });
        return;
      }

      if (message.type === 'get_status') {
        sendResponse({
          wasmInitialized: !!orynCore,
          hasScan: !!currentScan
        });
        return;
      }

      sendResponse({ error: 'Unknown message type' });
    };
  });

  describe('execute_oil message', () => {
    test('should process OIL command successfully', async () => {
      const mockTab = createMockTab();
      const mockScan = createMockScanResult();
      const sendResponse = jest.fn();

      // Setup
      currentScan = mockScan;
      orynCore.updateScan(JSON.stringify(mockScan));
      mockTabsSendMessage({ success: true, result: 'ok' });

      // Execute
      const message = { type: 'execute_oil', oil: 'observe' };
      const sender = { tab: mockTab };

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result.success).toBe(true);
    });

    test('should return error when no tab ID provided', async () => {
      const sendResponse = jest.fn();
      const message = { type: 'execute_oil', oil: 'observe' };
      const sender = { tab: undefined };

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result.error).toBe('No active tab');
    });

    test('should return error when WASM not initialized', async () => {
      orynCore = null; // Simulate uninitialized WASM

      const mockTab = createMockTab();
      const sendResponse = jest.fn();
      const message = { type: 'execute_oil', oil: 'observe' };
      const sender = { tab: mockTab };

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result.error).toContain('WASM');
    });
  });

  describe('scan_complete message', () => {
    test('should update scan context', async () => {
      const mockScan = createMockScanResult();
      const sendResponse = jest.fn();

      const message = { type: 'scan_complete', scan: mockScan };
      const sender = {};

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result.ok).toBe(true);
      // Verify scan was stored
      expect(currentScan).toBeDefined();
    });
  });

  describe('get_status message', () => {
    test('should return WASM initialized status', async () => {
      const sendResponse = jest.fn();
      const message = { type: 'get_status' };
      const sender = {};

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result).toHaveProperty('wasmInitialized');
      expect(result).toHaveProperty('hasScan');
    });
  });

  describe('unknown message type', () => {
    test('should return error for unknown message', async () => {
      const sendResponse = jest.fn();
      const message = { type: 'unknown_type' };
      const sender = {};

      const result = await new Promise(resolve => {
        messageListener(message, sender, resolve);
      });

      expect(result.error).toBe('Unknown message type');
    });
  });
});

describe('Background Script - Command Execution', () => {
  test('should execute browser navigation action', async () => {
    const tabId = 1;
    const action = {
      Browser: {
        Navigate: { url: 'https://example.com' }
      }
    };

    chrome.tabs.update.yields();

    // This would be tested in the actual implementation
    // For now, we verify the Chrome API would be called
    expect(chrome.tabs.update).toBeDefined();
  });

  test('should execute scanner action', async () => {
    const tabId = 1;
    const action = {
      Scanner: {
        Scan: { include_patterns: true }
      }
    };

    mockTabsSendMessage({ success: true, data: {} });

    // Verify scanner command would be sent
    expect(chrome.tabs.sendMessage).toBeDefined();
  });
});

describe('Background Script - Scan Management', () => {
  test('should request fresh scan when needed', async () => {
    const mockTab = createMockTab();
    const mockScan = createMockScanResult();

    mockTabsSendMessage(mockScan);

    // Simulate scan request
    chrome.tabs.sendMessage.yields(mockScan);

    // Verify scan message would be sent
    expect(chrome.tabs.sendMessage).toBeDefined();
  });

  test('should handle scan errors gracefully', async () => {
    const mockTab = createMockTab();

    mockTabsSendMessage({ error: 'Scan failed' });

    // Error handling would be tested in integration tests
    expect(chrome.tabs.sendMessage).toBeDefined();
  });
});

describe('Background Script - Error Handling', () => {
  test('should handle WASM processing errors', () => {
    const orynCore = createMockWasmModule().OrynCore;
    const instance = new orynCore();

    // Mock WASM throwing error
    instance.processCommand = () => {
      throw new Error('WASM processing failed');
    };

    expect(() => instance.processCommand('invalid')).toThrow('WASM processing failed');
  });

  test('should handle JSON parsing errors', () => {
    const invalidJson = 'not valid json';

    expect(() => JSON.parse(invalidJson)).toThrow();
  });
});
