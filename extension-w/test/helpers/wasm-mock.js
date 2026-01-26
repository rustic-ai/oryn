/**
 * More realistic WASM module mock for integration testing
 * Simulates actual WASM behavior more closely than unit test mocks
 */

class OrynCore {
  constructor() {
    this.scan = null;
  }

  updateScan(scanJson) {
    if (!scanJson || typeof scanJson !== 'string') {
      throw new Error('Invalid scan JSON: must be a string');
    }

    try {
      this.scan = JSON.parse(scanJson);
    } catch (e) {
      throw new Error(`Failed to parse scan JSON: ${e.message}`);
    }

    // Validate scan structure
    if (!this.scan.page || !this.scan.elements || !Array.isArray(this.scan.elements)) {
      throw new Error('Invalid scan structure: missing required fields');
    }
  }

  processCommand(oil) {
    if (!oil || typeof oil !== 'string') {
      throw new Error('Invalid command: must be a non-empty string');
    }

    const trimmed = oil.trim();
    if (!trimmed) {
      throw new Error('Invalid command: cannot be empty or whitespace');
    }

    if (!this.scan) {
      throw new Error('No scan loaded. Call updateScan() first.');
    }

    // Simulate parsing and translation
    try {
      const result = this._parseAndTranslate(trimmed);
      return JSON.stringify(result);
    } catch (e) {
      throw new Error(`Command processing failed: ${e.message}`);
    }
  }

  _parseAndTranslate(oil) {
    // Simulate basic OIL parsing and translation
    const lowerOil = oil.toLowerCase();

    if (lowerOil === 'observe') {
      return {
        Resolved: {
          Scanner: {
            Scan: { include_patterns: true }
          }
        }
      };
    }

    if (lowerOil.startsWith('goto ')) {
      const urlMatch = oil.match(/goto\s+"([^"]+)"/);
      if (!urlMatch) {
        throw new Error('Invalid goto syntax: expected goto "url"');
      }
      return {
        Resolved: {
          Browser: {
            Navigate: { url: urlMatch[1] }
          }
        }
      };
    }

    if (lowerOil.startsWith('click ')) {
      const targetMatch = oil.match(/click\s+"([^"]+)"/);
      if (!targetMatch) {
        throw new Error('Invalid click syntax: expected click "target"');
      }
      return {
        Resolved: {
          Scanner: {
            Click: { target: { Text: targetMatch[1] } }
          }
        }
      };
    }

    if (lowerOil.startsWith('type ')) {
      const typeMatch = oil.match(/type\s+"([^"]*)"\s+"([^"]*)"/);
      if (!typeMatch) {
        throw new Error('Invalid type syntax: expected type "target" "value"');
      }
      return {
        Resolved: {
          Scanner: {
            Type: {
              target: { Text: typeMatch[1] },
              value: typeMatch[2]
            }
          }
        }
      };
    }

    if (lowerOil === 'submit') {
      return {
        Resolved: {
          Scanner: {
            Submit: {}
          }
        }
      };
    }

    throw new Error(`Unknown command: ${oil.split(' ')[0]}`);
  }

  static getVersion() {
    return '0.1.0';
  }
}

module.exports = { OrynCore };
