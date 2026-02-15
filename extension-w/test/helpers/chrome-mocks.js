// Helper functions for mocking Chrome APIs

const DEFAULT_ELEMENT = {
  id: 1,
  selector: '#submit',
  element_type: 'button',
  text: 'Submit',
  attributes: {},
  rect: { x: 0, y: 0, width: 100, height: 30 },
  label: null,
  placeholder: null,
  value: null,
  checked: null,
  href: null
};

/**
 * Create a mock ScanResult
 */
function createMockScanResult(options = {}) {
  const elements = options.elements || [DEFAULT_ELEMENT];
  const elementCount = elements.length;

  return {
    page: {
      url: options.url || 'https://example.com',
      title: options.title || 'Test Page',
      viewport: { width: 1920, height: 1080 },
      scroll: { x: 0, y: 0 }
    },
    elements,
    stats: {
      total: elementCount,
      scanned: elementCount
    },
    patterns: options.patterns || null,
    changes: null,
    available_intents: null
  };
}

/**
 * Mock chrome.runtime.sendMessage with custom response
 */
function mockRuntimeSendMessage(response) {
  global.chrome.runtime.sendMessage.yields(response);
}

/**
 * Mock chrome.tabs.sendMessage with custom response
 */
function mockTabsSendMessage(response) {
  global.chrome.tabs.sendMessage.yields(response);
}

/**
 * Mock chrome.tabs.query with specific tabs
 */
function mockTabsQuery(tabs) {
  global.chrome.tabs.query.yields(tabs);
}

const DEFAULT_TAB = {
  id: 1,
  url: 'https://example.com',
  title: 'Test Page',
  active: true,
  windowId: 1
};

/**
 * Create a mock Chrome tab
 */
function createMockTab(options = {}) {
  return { ...DEFAULT_TAB, ...options };
}

/**
 * Create a mock WASM module
 */
function createMockWasmModule() {
  return {
    OrynCore: class {
      constructor() {
        this.scan = null;
      }

      updateScan(scanJson) {
        this.scan = JSON.parse(scanJson);
      }

      processCommand(oil) {
        return JSON.stringify({
          Resolved: {
            Scanner: {
              Scan: { include_patterns: true }
            }
          }
        });
      }

      static getVersion() {
        return '0.1.0';
      }
    }
  };
}

/**
 * Wait for async operations
 */
function waitFor(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

module.exports = {
  createMockScanResult,
  mockRuntimeSendMessage,
  mockTabsSendMessage,
  mockTabsQuery,
  createMockTab,
  createMockWasmModule,
  waitFor
};
