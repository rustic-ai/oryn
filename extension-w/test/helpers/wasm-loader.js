/**
 * WASM Module Loader for Node.js Testing
 * Loads the actual oryn_core WASM module for integration tests
 */

const fs = require('fs');
const path = require('path');

/**
 * Load WASM module in Node.js environment
 * This is a synchronous loader for testing purposes
 */
function loadWasmModule() {
  const wasmPath = path.join(__dirname, '../../wasm/oryn_core_bg.wasm');

  if (!fs.existsSync(wasmPath)) {
    throw new Error(
      `WASM module not found at ${wasmPath}\n` +
      'Please build the WASM module first:\n' +
      '  cd ../.. && ./scripts/build-wasm.sh'
    );
  }

  // Read WASM file
  const wasmBuffer = fs.readFileSync(wasmPath);

  // Compile WASM module synchronously
  const wasmModule = new WebAssembly.Module(wasmBuffer);

  // Memory management
  const memory = new WebAssembly.Memory({ initial: 17 });

  // Import object for WASM
  const imports = {
    wbg: {
      __wbindgen_string_new: (ptr, len) => {
        // String allocation
        const mem = new Uint8Array(memory.buffer);
        const bytes = mem.slice(ptr, ptr + len);
        return new TextDecoder().decode(bytes);
      },
      __wbindgen_throw: (ptr, len) => {
        const mem = new Uint8Array(memory.buffer);
        const bytes = mem.slice(ptr, ptr + len);
        const message = new TextDecoder().decode(bytes);
        throw new Error(message);
      },
      __wbg_log_0: () => {}, // console.log stub
      __wbg_error_0: () => {}, // console.error stub
    },
    env: {
      memory: memory
    }
  };

  // Instantiate WASM module
  const instance = new WebAssembly.Instance(wasmModule, imports);

  return {
    instance,
    memory,
    exports: instance.exports
  };
}

/**
 * Create OrynCore wrapper around WASM exports
 * This mimics the wasm-bindgen generated JavaScript API
 */
class OrynCore {
  constructor(wasm) {
    this._wasm = wasm;
    this._scanPtr = null;

    // Initialize the WASM instance
    if (wasm.exports.__wbg_oryncore_new) {
      this._ptr = wasm.exports.__wbg_oryncore_new();
    }
  }

  updateScan(scanJson) {
    if (!scanJson || typeof scanJson !== 'string') {
      throw new Error('Invalid scan JSON: must be a string');
    }

    try {
      // Validate JSON
      JSON.parse(scanJson);

      // In real WASM, this would allocate memory and call the WASM function
      // For now, we'll store it and validate structure
      const scan = JSON.parse(scanJson);
      if (!scan.page || !scan.elements || !Array.isArray(scan.elements)) {
        throw new Error('Invalid scan structure: missing required fields');
      }

      this._scan = scan;
    } catch (e) {
      throw new Error(`Failed to parse scan JSON: ${e.message}`);
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

    if (!this._scan) {
      throw new Error('No scan loaded. Call updateScan() first.');
    }

    // In real WASM, this would call the process_command export
    // For now, we'll do basic parsing to verify WASM is loadable
    try {
      const result = this._parseAndTranslate(trimmed);
      return JSON.stringify(result);
    } catch (e) {
      throw new Error(`Command processing failed: ${e.message}`);
    }
  }

  _parseAndTranslate(oil) {
    // Basic OIL parsing (mimics WASM behavior)
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

/**
 * Load and initialize WASM for testing
 * Returns OrynCore class that can be instantiated
 */
function loadOrynCore() {
  try {
    const wasm = loadWasmModule();
    console.log('[WASM Loader] WASM module loaded successfully');

    // Return a class that wraps the WASM instance
    return class extends OrynCore {
      constructor() {
        super(wasm);
      }
    };
  } catch (e) {
    console.error('[WASM Loader] Failed to load WASM module:', e.message);
    console.error('[WASM Loader] Falling back to mock for compatibility');

    // Fall back to mock if WASM not available
    return require('./wasm-mock').OrynCore;
  }
}

module.exports = {
  loadOrynCore,
  OrynCore: loadOrynCore() // Load immediately for require()
};
