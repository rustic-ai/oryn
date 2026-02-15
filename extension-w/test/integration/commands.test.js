/**
 * Integration tests for command processing
 * Tests end-to-end command flow from parsing to action execution
 */

const {
  createMockScanResult,
  mockTabsSendMessage,
  createMockTab
} = require('../helpers/chrome-mocks');

const { OrynCore } = require('../helpers/wasm-mock');

describe('Command Processing Integration', () => {
  let orynCore;
  let mockTab;

  beforeEach(() => {
    orynCore = new OrynCore();
    mockTab = createMockTab();

    // Setup scan
    const scan = createMockScanResult({
      elements: [
        { id: 1, selector: '#submit', element_type: 'button', text: 'Submit', rect: { x: 100, y: 200, width: 80, height: 30 } },
        { id: 2, selector: '#cancel', element_type: 'button', text: 'Cancel', rect: { x: 200, y: 200, width: 80, height: 30 } },
        { id: 3, selector: '#email', element_type: 'input', placeholder: 'Email', rect: { x: 100, y: 100, width: 200, height: 30 } },
        { id: 4, selector: '#password', element_type: 'input', placeholder: 'Password', rect: { x: 100, y: 150, width: 200, height: 30 } }
      ]
    });
    orynCore.updateScan(JSON.stringify(scan));
  });

  describe('Browser Commands', () => {
    test('should process goto command', () => {
      const result = orynCore.processCommand('goto "https://example.com"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Browser.Navigate.url).toBe('https://example.com');
    });

    test('should handle URLs with query parameters', () => {
      const result = orynCore.processCommand('goto "https://example.com?q=test&lang=en"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Browser.Navigate.url).toBe('https://example.com?q=test&lang=en');
    });

    test('should handle URLs with fragments', () => {
      const result = orynCore.processCommand('goto "https://example.com#section"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Browser.Navigate.url).toBe('https://example.com#section');
    });
  });

  describe('Scanner Commands', () => {
    test('should process observe command', () => {
      const result = orynCore.processCommand('observe');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Scanner.Scan).toBeDefined();
      expect(parsed.Resolved.Scanner.Scan.include_patterns).toBe(true);
    });

    test('should process click with text target', () => {
      const result = orynCore.processCommand('click "Submit"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Scanner.Click).toBeDefined();
      expect(parsed.Resolved.Scanner.Click.target.Text).toBe('Submit');
    });

    test('should process type command', () => {
      const result = orynCore.processCommand('type "Email" "test@example.com"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Scanner.Type).toBeDefined();
      expect(parsed.Resolved.Scanner.Type.target.Text).toBe('Email');
      expect(parsed.Resolved.Scanner.Type.value).toBe('test@example.com');
    });

    test('should process submit command', () => {
      const result = orynCore.processCommand('submit');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Scanner.Submit).toBeDefined();
    });
  });

  describe('Command Variations', () => {
    test('should handle click with different targets', () => {
      const commands = [
        'click "Submit"',
        'click "Cancel"',
        'click "Email"'
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        expect(JSON.parse(result)).toHaveProperty('Resolved');
      });
    });

    test('should handle type with different values', () => {
      const commands = [
        'type "Email" "user@test.com"',
        'type "Password" "secret123"',
        'type "Email" ""'  // Empty value
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        expect(JSON.parse(result)).toHaveProperty('Resolved');
      });
    });

    test('should handle special characters in values', () => {
      const result = orynCore.processCommand('type "Email" "test+filter@example.com"');
      const parsed = JSON.parse(result);

      expect(parsed.Resolved.Scanner.Type.value).toBe('test+filter@example.com');
    });
  });

  describe('Error Cases', () => {
    test('should reject malformed goto', () => {
      expect(() => orynCore.processCommand('goto example.com')).toThrow(/goto/);
    });

    test('should reject malformed click', () => {
      expect(() => orynCore.processCommand('click Submit')).toThrow(/click/);
    });

    test('should reject malformed type', () => {
      expect(() => orynCore.processCommand('type Email test@example.com')).toThrow(/type/);
    });

    test('should reject unknown commands', () => {
      expect(() => orynCore.processCommand('unknown "target"')).toThrow(/unknown/i);
    });
  });

  describe('Command Sequences', () => {
    test('should process login sequence', () => {
      const commands = [
        'observe',
        'type "Email" "user@example.com"',
        'type "Password" "secret123"',
        'click "Submit"'
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        const parsed = JSON.parse(result);
        expect(parsed).toHaveProperty('Resolved');
      });
    });

    test('should process navigation sequence', () => {
      const commands = [
        'goto "https://example.com"',
        'observe',
        'click "Login"'
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        const parsed = JSON.parse(result);
        expect(parsed).toHaveProperty('Resolved');
      });
    });

    test('should handle scan updates mid-sequence', () => {
      // Initial commands
      orynCore.processCommand('observe');
      orynCore.processCommand('click "Submit"');

      // Update scan (simulating page change)
      const newScan = createMockScanResult({
        url: 'https://example.com/page2',
        elements: [
          { id: 5, selector: '#next', element_type: 'button', text: 'Next' }
        ]
      });
      orynCore.updateScan(JSON.stringify(newScan));

      // Continue with new scan
      const result = orynCore.processCommand('click "Next"');
      expect(JSON.parse(result)).toHaveProperty('Resolved');
    });
  });

  describe('Integration with Background Script', () => {
    test('should simulate background message flow', async () => {
      // Simulate background.js receiving execute_oil message
      const message = {
        type: 'execute_oil',
        oil: 'click "Submit"'
      };

      // Process command
      const resultJson = orynCore.processCommand(message.oil);
      const result = JSON.parse(resultJson);

      // Verify action structure
      expect(result.Resolved).toBeDefined();
      expect(result.Resolved.Scanner).toBeDefined();
    });

    test('should handle scan_complete message flow', () => {
      const newScan = createMockScanResult({
        url: 'https://updated.com',
        elements: [
          { id: 10, selector: '#new-button', element_type: 'button', text: 'New' }
        ]
      });

      // Simulate scan_complete message
      const message = {
        type: 'scan_complete',
        scan: newScan
      };

      // Update scan
      orynCore.updateScan(JSON.stringify(message.scan));

      // Should work with updated scan
      const result = orynCore.processCommand('observe');
      expect(JSON.parse(result)).toHaveProperty('Resolved');
    });
  });

  describe('Normalization', () => {
    test('should handle commands with extra whitespace', () => {
      const commands = [
        '  observe  ',
        'click   "Submit"  ',
        '  goto "https://example.com"  '
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        expect(JSON.parse(result)).toHaveProperty('Resolved');
      });
    });

    test('should handle case variations', () => {
      const commands = [
        'OBSERVE',
        'Observe',
        'oBsErVe'
      ];

      commands.forEach(cmd => {
        const result = orynCore.processCommand(cmd);
        expect(JSON.parse(result)).toHaveProperty('Resolved');
      });
    });
  });

  describe('Performance', () => {
    test('should process commands quickly', () => {
      const start = Date.now();

      for (let i = 0; i < 100; i++) {
        orynCore.processCommand('observe');
      }

      const duration = Date.now() - start;

      // Should process 100 commands in less than 100ms (1ms average)
      expect(duration).toBeLessThan(100);
    });

    test('should handle large scans', () => {
      // Create scan with 1000 elements
      const elements = [];
      for (let i = 0; i < 1000; i++) {
        elements.push({
          id: i,
          selector: `#elem-${i}`,
          element_type: 'button',
          text: `Button ${i}`,
          rect: { x: 0, y: i * 10, width: 100, height: 30 }
        });
      }

      const largeScan = createMockScanResult({ elements });
      orynCore.updateScan(JSON.stringify(largeScan));

      const start = Date.now();
      orynCore.processCommand('observe');
      const duration = Date.now() - start;

      // Should handle large scan in reasonable time
      expect(duration).toBeLessThan(50);
    });
  });
});
