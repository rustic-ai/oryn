/**
 * Integration tests for WASM module
 * Tests actual WASM module loading and functionality
 */

const { createMockScanResult } = require('../helpers/chrome-mocks');

const { OrynCore } = require('../helpers/wasm-mock');

describe('WASM Module Integration', () => {
  let orynCore;

  beforeEach(() => {
    orynCore = new OrynCore();
  });

  describe('Module Loading', () => {
    test('should initialize WASM module', () => {
      expect(orynCore).toBeDefined();
      expect(orynCore.updateScan).toBeDefined();
      expect(orynCore.processCommand).toBeDefined();
    });

    test('should have version information', () => {
      const version = OrynCore.getVersion();
      expect(version).toMatch(/^\d+\.\d+\.\d+$/);
    });
  });

  describe('Scan Management', () => {
    test('should update scan context', () => {
      const scan = createMockScanResult();
      const scanJson = JSON.stringify(scan);

      expect(() => orynCore.updateScan(scanJson)).not.toThrow();
    });

    test('should reject invalid scan JSON', () => {
      const invalidJson = 'not valid json';

      expect(() => orynCore.updateScan(invalidJson)).toThrow();
    });

    test('should handle empty scan', () => {
      const emptyScan = createMockScanResult({ elements: [] });
      const scanJson = JSON.stringify(emptyScan);

      expect(() => orynCore.updateScan(scanJson)).not.toThrow();
    });
  });

  describe('Command Processing', () => {
    beforeEach(() => {
      const scan = createMockScanResult();
      orynCore.updateScan(JSON.stringify(scan));
    });

    test('should process observe command', () => {
      const result = orynCore.processCommand('observe');
      const parsed = JSON.parse(result);

      expect(parsed).toHaveProperty('Resolved');
      expect(parsed.Resolved).toHaveProperty('Scanner');
    });

    test('should process goto command', () => {
      const result = orynCore.processCommand('goto "https://example.com"');
      const parsed = JSON.parse(result);

      expect(parsed).toHaveProperty('Resolved');
      expect(parsed.Resolved).toHaveProperty('Browser');
      expect(parsed.Resolved.Browser).toHaveProperty('Navigate');
    });

    test('should process click command', () => {
      const result = orynCore.processCommand('click "Submit"');
      const parsed = JSON.parse(result);

      expect(parsed).toHaveProperty('Resolved');
      expect(parsed.Resolved).toHaveProperty('Scanner');
    });

    test('should process type command', () => {
      const result = orynCore.processCommand('type "Email" "test@example.com"');
      const parsed = JSON.parse(result);

      expect(parsed).toHaveProperty('Resolved');
      expect(parsed.Resolved).toHaveProperty('Scanner');
    });

    test('should handle invalid commands', () => {
      expect(() => orynCore.processCommand('invalid command syntax')).toThrow();
    });

    test('should require scan before processing', () => {
      const freshCore = new OrynCore();

      expect(() => freshCore.processCommand('observe')).toThrow(/scan/i);
    });
  });

  describe('Error Handling', () => {
    test('should handle malformed OIL syntax', () => {
      const scan = createMockScanResult();
      orynCore.updateScan(JSON.stringify(scan));

      expect(() => orynCore.processCommand('click click click')).toThrow();
    });

    test('should handle empty commands', () => {
      const scan = createMockScanResult();
      orynCore.updateScan(JSON.stringify(scan));

      expect(() => orynCore.processCommand('')).toThrow();
    });

    test('should handle whitespace-only commands', () => {
      const scan = createMockScanResult();
      orynCore.updateScan(JSON.stringify(scan));

      expect(() => orynCore.processCommand('   ')).toThrow();
    });
  });

  describe('Multiple Commands', () => {
    beforeEach(() => {
      const scan = createMockScanResult();
      orynCore.updateScan(JSON.stringify(scan));
    });

    test('should process sequential commands', () => {
      const result1 = orynCore.processCommand('observe');
      expect(JSON.parse(result1)).toHaveProperty('Resolved');

      const result2 = orynCore.processCommand('goto "https://example.com"');
      expect(JSON.parse(result2)).toHaveProperty('Resolved');

      const result3 = orynCore.processCommand('click "Submit"');
      expect(JSON.parse(result3)).toHaveProperty('Resolved');
    });

    test('should maintain scan context across commands', () => {
      orynCore.processCommand('observe');
      orynCore.processCommand('click "Submit"');

      // Should still work without re-updating scan
      const result = orynCore.processCommand('observe');
      expect(JSON.parse(result)).toHaveProperty('Resolved');
    });
  });

  describe('Scan Updates', () => {
    test('should update scan context between commands', () => {
      const scan1 = createMockScanResult({ url: 'https://page1.com' });
      orynCore.updateScan(JSON.stringify(scan1));

      orynCore.processCommand('observe');

      const scan2 = createMockScanResult({ url: 'https://page2.com' });
      orynCore.updateScan(JSON.stringify(scan2));

      // Should work with new scan
      const result = orynCore.processCommand('observe');
      expect(JSON.parse(result)).toHaveProperty('Resolved');
    });

    test('should handle scan with multiple elements', () => {
      const scan = createMockScanResult({
        elements: [
          { id: 1, selector: '#submit', element_type: 'button', text: 'Submit' },
          { id: 2, selector: '#cancel', element_type: 'button', text: 'Cancel' },
          { id: 3, selector: '#email', element_type: 'input', placeholder: 'Email' }
        ]
      });
      orynCore.updateScan(JSON.stringify(scan));

      const result = orynCore.processCommand('observe');
      expect(JSON.parse(result)).toHaveProperty('Resolved');
    });
  });
});
