// Jest setup file - runs before all tests
const chrome = require('sinon-chrome');

// Set up global Chrome API mock
global.chrome = chrome;

// Add sidePanel API (sinon-chrome doesn't include it by default)
if (!chrome.sidePanel) {
  chrome.sidePanel = {
    open: jest.fn().mockResolvedValue(undefined)
  };
}

// Mock console methods to reduce noise in tests
global.console = {
  ...console,
  log: jest.fn(),
  debug: jest.fn(),
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn(),
};

// Reset mocks before each test
beforeEach(() => {
  chrome.flush();
  jest.clearAllMocks();
});

// Clean up after each test
afterEach(() => {
  chrome.reset();
});

// Add custom matchers
expect.extend({
  toHaveBeenCalledWithMessage(received, expectedType, expectedData) {
    const calls = received.mock.calls;
    const pass = calls.some(call => {
      const message = call[0];
      return message.type === expectedType &&
             (!expectedData || JSON.stringify(message).includes(JSON.stringify(expectedData)));
    });

    return {
      pass,
      message: () => pass
        ? `Expected not to be called with type ${expectedType}`
        : `Expected to be called with type ${expectedType}, but was not`
    };
  }
});
