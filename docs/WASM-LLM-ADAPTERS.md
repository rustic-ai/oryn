# WASM LLM Adapter Implementation Guide

This document provides implementation examples for each WASM-based LLM framework adapter for Extension-W.

## 1. WebLLM Adapter (Recommended)

### Installation
```bash
npm install @mlc-ai/web-llm
```

### Implementation

```javascript
// extension-w/llm/webllm_adapter.js

import * as webllm from "@mlc-ai/web-llm";
import { LLMAdapter } from './llm_adapter.js';

export class WebLLMAdapter extends LLMAdapter {
  constructor() {
    super();
    this.engine = null;
    this.modelId = null;
    this.downloadProgress = 0;
  }

  static async isAvailable() {
    try {
      // Check for WebGPU support
      if (!navigator.gpu) {
        console.log('WebGPU not available');
        return false;
      }
      const adapter = await navigator.gpu.requestAdapter();
      return adapter !== null;
    } catch (e) {
      return false;
    }
  }

  async initialize(modelId = "Phi-3-mini-4k-instruct-q4f16_1", options = {}) {
    this.modelId = modelId;

    try {
      // Create engine with progress callback
      this.engine = await webllm.CreateMLCEngine(modelId, {
        initProgressCallback: (progress) => {
          this.downloadProgress = progress.progress || 0;
          console.log(`Loading model: ${Math.round(this.downloadProgress * 100)}%`);

          // Emit progress event for UI
          if (options.onProgress) {
            options.onProgress({
              loaded: progress.loaded,
              total: progress.total,
              progress: this.downloadProgress
            });
          }
        }
      });

      console.log('WebLLM engine initialized with', modelId);
      return true;
    } catch (error) {
      console.error('Failed to initialize WebLLM:', error);
      throw new Error(`WebLLM initialization failed: ${error.message}`);
    }
  }

  async prompt(messages, options = {}) {
    if (!this.engine) {
      throw new Error('WebLLM engine not initialized');
    }

    try {
      // Format messages for OpenAI-compatible API
      const formattedMessages = messages.map(msg => ({
        role: msg.role || 'user',
        content: msg.content
      }));

      const response = await this.engine.chat.completions.create({
        messages: formattedMessages,
        temperature: options.temperature || 0.7,
        max_tokens: options.maxTokens || 512,
        stream: false
      });

      return response.choices[0].message.content;
    } catch (error) {
      console.error('WebLLM prompt failed:', error);
      throw error;
    }
  }

  async stream(messages, options = {}) {
    if (!this.engine) {
      throw new Error('WebLLM engine not initialized');
    }

    const formattedMessages = messages.map(msg => ({
      role: msg.role || 'user',
      content: msg.content
    }));

    const stream = await this.engine.chat.completions.create({
      messages: formattedMessages,
      temperature: options.temperature || 0.7,
      max_tokens: options.maxTokens || 512,
      stream: true
    });

    // Return async generator for streaming
    return (async function* () {
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content;
        if (content) {
          yield content;
        }
      }
    })();
  }

  getCapabilities() {
    return {
      name: 'WebLLM',
      type: 'local',
      maxTokens: 4096,
      streaming: true,
      local: true,
      requiresWebGPU: true,
      models: [
        'Llama-3-8B-Instruct-q4f16_1',      // 4.5GB, best quality
        'Phi-3-mini-4k-instruct-q4f16_1',   // 2.2GB, fast
        'Gemma-2B-it-q4f16_1',              // 1.5GB, fastest
        'Mistral-7B-Instruct-v0.2-q4f16_1'  // 4.1GB, good
      ]
    };
  }

  getStatus() {
    return {
      ready: this.engine !== null,
      model: this.modelId,
      downloadProgress: this.downloadProgress,
      error: null
    };
  }

  async unload() {
    if (this.engine) {
      await this.engine.unload();
      this.engine = null;
    }
  }
}
```

### Usage Example
```javascript
const adapter = new WebLLMAdapter();

if (await WebLLMAdapter.isAvailable()) {
  await adapter.initialize('Phi-3-mini-4k-instruct-q4f16_1', {
    onProgress: (p) => console.log(`Download: ${p.progress * 100}%`)
  });

  const response = await adapter.prompt([
    { role: 'system', content: 'You are a web automation assistant.' },
    { role: 'user', content: 'What action should I take to search?' }
  ]);

  console.log(response);
}
```

---

## 2. wllama Adapter (CPU Fallback)

### Installation
```bash
npm install @wllama/wllama
```

### Implementation

```javascript
// extension-w/llm/wllama_adapter.js

import { Wllama } from '@wllama/wllama';
import { LLMAdapter } from './llm_adapter.js';

export class WllamaAdapter extends LLMAdapter {
  constructor() {
    super();
    this.wllama = null;
    this.modelUrl = null;
    this.tokenizer = null;
  }

  static async isAvailable() {
    // wllama works everywhere WASM is supported
    try {
      return typeof WebAssembly !== 'undefined';
    } catch {
      return false;
    }
  }

  async initialize(modelUrl, options = {}) {
    this.modelUrl = modelUrl;

    try {
      this.wllama = new Wllama({
        // Use multi-threading if available
        useMultiThread: options.multiThread !== false,
        nThreads: options.nThreads || navigator.hardwareConcurrency || 4
      });

      await this.wllama.loadModel({
        model: modelUrl,
        // Progress callback
        progressCallback: (progress) => {
          if (options.onProgress) {
            options.onProgress({
              loaded: progress.loaded,
              total: progress.total,
              progress: progress.loaded / progress.total
            });
          }
        }
      });

      console.log('wllama model loaded:', modelUrl);
      return true;
    } catch (error) {
      console.error('Failed to initialize wllama:', error);
      throw new Error(`wllama initialization failed: ${error.message}`);
    }
  }

  async prompt(messages, options = {}) {
    if (!this.wllama) {
      throw new Error('wllama not initialized');
    }

    try {
      // Format messages into a single prompt
      const prompt = this.formatMessages(messages);

      const result = await this.wllama.createCompletion({
        prompt: prompt,
        n_predict: options.maxTokens || 512,
        temperature: options.temperature || 0.7,
        top_k: options.topK || 40,
        top_p: options.topP || 0.9
      });

      return result.trim();
    } catch (error) {
      console.error('wllama prompt failed:', error);
      throw error;
    }
  }

  async stream(messages, options = {}) {
    if (!this.wllama) {
      throw new Error('wllama not initialized');
    }

    const prompt = this.formatMessages(messages);

    const stream = await this.wllama.createCompletion({
      prompt: prompt,
      n_predict: options.maxTokens || 512,
      temperature: options.temperature || 0.7,
      top_k: options.topK || 40,
      top_p: options.topP || 0.9,
      stream: true
    });

    // Return async generator
    return (async function* () {
      for await (const chunk of stream) {
        yield chunk;
      }
    })();
  }

  formatMessages(messages) {
    // Simple format: System\n\nUser: <msg>\nAssistant:
    let prompt = '';

    for (const msg of messages) {
      if (msg.role === 'system') {
        prompt += msg.content + '\n\n';
      } else if (msg.role === 'user') {
        prompt += `User: ${msg.content}\n`;
      } else if (msg.role === 'assistant') {
        prompt += `Assistant: ${msg.content}\n`;
      }
    }

    prompt += 'Assistant:';
    return prompt;
  }

  getCapabilities() {
    return {
      name: 'wllama',
      type: 'local',
      maxTokens: 2048,
      streaming: true,
      local: true,
      requiresWebGPU: false,
      models: [
        'TinyLlama-1.1B-Chat-v1.0 (Q4)',     // 669MB
        'Phi-2 (Q4_K_M)',                    // 1.6GB
        'Llama-2-7B-Chat (Q4_K_M)',          // 3.8GB
      ]
    };
  }

  getStatus() {
    return {
      ready: this.wllama !== null,
      model: this.modelUrl,
      error: null
    };
  }

  async unload() {
    if (this.wllama) {
      await this.wllama.exit();
      this.wllama = null;
    }
  }
}
```

### Model URLs
```javascript
const GGUF_MODELS = {
  'tinyllama': 'https://huggingface.co/TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF/resolve/main/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf',
  'phi2': 'https://huggingface.co/TheBloke/phi-2-GGUF/resolve/main/phi-2.Q4_K_M.gguf',
  'llama2-7b': 'https://huggingface.co/TheBloke/Llama-2-7B-Chat-GGUF/resolve/main/llama-2-7b-chat.Q4_K_M.gguf'
};
```

---

## 3. ONNX Runtime Web Adapter

### Installation
```bash
npm install onnxruntime-web
```

### Implementation

```javascript
// extension-w/llm/onnxweb_adapter.js

import * as ort from 'onnxruntime-web';
import { LLMAdapter } from './llm_adapter.js';

export class ONNXWebAdapter extends LLMAdapter {
  constructor() {
    super();
    this.session = null;
    this.tokenizer = null;
    this.modelPath = null;
  }

  static async isAvailable() {
    // Try to detect WebGPU support
    try {
      const hasWebGPU = 'gpu' in navigator;
      return true; // ONNX works with WASM fallback
    } catch {
      return true;
    }
  }

  async initialize(modelPath, tokenizerPath, options = {}) {
    this.modelPath = modelPath;

    try {
      // Configure execution providers (try WebGPU first, fallback to WASM)
      const executionProviders = [];

      if (await this.hasWebGPU()) {
        executionProviders.push('webgpu');
      }

      executionProviders.push('wasm');

      // Set WASM threads
      ort.env.wasm.numThreads = options.numThreads || navigator.hardwareConcurrency || 4;
      ort.env.wasm.simd = true;

      // Load model
      this.session = await ort.InferenceSession.create(modelPath, {
        executionProviders: executionProviders
      });

      // Load tokenizer (custom implementation or use tokenizers library)
      this.tokenizer = await this.loadTokenizer(tokenizerPath);

      console.log('ONNX Runtime Web initialized with', executionProviders);
      return true;
    } catch (error) {
      console.error('Failed to initialize ONNX Runtime Web:', error);
      throw new Error(`ONNX initialization failed: ${error.message}`);
    }
  }

  async hasWebGPU() {
    try {
      if (!navigator.gpu) return false;
      const adapter = await navigator.gpu.requestAdapter();
      return adapter !== null;
    } catch {
      return false;
    }
  }

  async loadTokenizer(path) {
    // Load tokenizer JSON (BPE, WordPiece, etc.)
    const response = await fetch(path);
    const config = await response.json();

    // Implement basic tokenizer or use @xenova/transformers tokenizer
    return {
      encode: (text) => {
        // Simple tokenization (replace with proper implementation)
        return text.split(' ').map(w => this.vocab[w] || 0);
      },
      decode: (ids) => {
        return ids.map(id => this.reverseVocab[id] || '').join(' ');
      }
    };
  }

  async prompt(messages, options = {}) {
    if (!this.session || !this.tokenizer) {
      throw new Error('ONNX Runtime Web not initialized');
    }

    try {
      // Format and tokenize input
      const prompt = this.formatMessages(messages);
      const inputIds = this.tokenizer.encode(prompt);

      // Create tensor
      const inputTensor = new ort.Tensor('int64', BigInt64Array.from(inputIds.map(id => BigInt(id))), [1, inputIds.length]);

      // Run inference
      const feeds = { input_ids: inputTensor };
      const results = await this.session.run(feeds);

      // Decode output
      const outputIds = Array.from(results.logits.data);
      const response = this.tokenizer.decode(outputIds);

      return response;
    } catch (error) {
      console.error('ONNX prompt failed:', error);
      throw error;
    }
  }

  formatMessages(messages) {
    let prompt = '';
    for (const msg of messages) {
      if (msg.role === 'system') {
        prompt += msg.content + '\n\n';
      } else if (msg.role === 'user') {
        prompt += `User: ${msg.content}\n`;
      } else if (msg.role === 'assistant') {
        prompt += `Assistant: ${msg.content}\n`;
      }
    }
    prompt += 'Assistant:';
    return prompt;
  }

  async stream(messages, options = {}) {
    // ONNX Runtime Web doesn't support native streaming
    // Implement token-by-token generation
    throw new Error('Streaming not yet implemented for ONNX Runtime Web');
  }

  getCapabilities() {
    return {
      name: 'ONNX Runtime Web',
      type: 'local',
      maxTokens: 2048,
      streaming: false,
      local: true,
      requiresWebGPU: false,
      models: [
        'Custom ONNX models (quantized INT8, FP16, INT4)'
      ]
    };
  }

  getStatus() {
    return {
      ready: this.session !== null,
      model: this.modelPath,
      error: null
    };
  }

  async unload() {
    if (this.session) {
      await this.session.release();
      this.session = null;
    }
  }
}
```

---

## 4. Unified LLM Manager

```javascript
// extension-w/llm/llm_manager.js

import { ChromeAIAdapter } from './chrome_ai_adapter.js';
import { WebLLMAdapter } from './webllm_adapter.js';
import { WllamaAdapter } from './wllama_adapter.js';
import { ONNXWebAdapter } from './onnxweb_adapter.js';

export class LLMManager {
  constructor() {
    this.adapters = new Map();
    this.currentAdapter = null;
    this.config = null;
  }

  async initialize() {
    // Load configuration from storage
    this.config = await chrome.storage.sync.get(['llmConfig']);

    // Auto-detect and register available adapters
    await this.detectAdapters();

    // Set active adapter from config or auto-select
    const preferredAdapter = this.config.llmConfig?.adapter || 'auto';
    await this.setActiveAdapter(preferredAdapter);
  }

  async detectAdapters() {
    console.log('Detecting available LLM adapters...');

    // Check Chrome AI
    if (await ChromeAIAdapter.isAvailable()) {
      this.adapters.set('chrome-ai', {
        name: 'Chrome AI',
        priority: 1,
        adapter: ChromeAIAdapter
      });
      console.log('✓ Chrome AI available');
    }

    // Check WebLLM (requires WebGPU)
    if (await WebLLMAdapter.isAvailable()) {
      this.adapters.set('webllm', {
        name: 'WebLLM',
        priority: 2,
        adapter: WebLLMAdapter
      });
      console.log('✓ WebLLM available (WebGPU detected)');
    }

    // wllama is always available (WASM)
    if (await WllamaAdapter.isAvailable()) {
      this.adapters.set('wllama', {
        name: 'wllama',
        priority: 3,
        adapter: WllamaAdapter
      });
      console.log('✓ wllama available');
    }

    // ONNX Runtime Web
    if (await ONNXWebAdapter.isAvailable()) {
      this.adapters.set('onnx', {
        name: 'ONNX Runtime Web',
        priority: 4,
        adapter: ONNXWebAdapter
      });
      console.log('✓ ONNX Runtime Web available');
    }

    console.log(`Detected ${this.adapters.size} adapter(s)`);
  }

  async setActiveAdapter(adapterName) {
    if (adapterName === 'auto') {
      // Auto-select best available adapter
      const sorted = Array.from(this.adapters.entries())
        .sort((a, b) => a[1].priority - b[1].priority);

      if (sorted.length === 0) {
        throw new Error('No LLM adapters available');
      }

      adapterName = sorted[0][0];
      console.log(`Auto-selected adapter: ${adapterName}`);
    }

    const adapterInfo = this.adapters.get(adapterName);
    if (!adapterInfo) {
      throw new Error(`Adapter not found: ${adapterName}`);
    }

    // Initialize adapter
    this.currentAdapter = new adapterInfo.adapter();

    // For Chrome AI, initialize immediately
    if (adapterName === 'chrome-ai') {
      await this.currentAdapter.initialize();
    }
    // For others, initialize on first use or with specific model

    console.log(`Active adapter set to: ${adapterName}`);
  }

  async prompt(messages, options = {}) {
    if (!this.currentAdapter) {
      throw new Error('No active LLM adapter');
    }

    return await this.currentAdapter.prompt(messages, options);
  }

  async stream(messages, options = {}) {
    if (!this.currentAdapter) {
      throw new Error('No active LLM adapter');
    }

    return await this.currentAdapter.stream(messages, options);
  }

  getAvailableAdapters() {
    return Array.from(this.adapters.entries()).map(([key, value]) => ({
      id: key,
      name: value.name,
      priority: value.priority,
      capabilities: value.adapter.prototype.getCapabilities()
    }));
  }

  getCurrentAdapter() {
    return this.currentAdapter;
  }

  getStatus() {
    return {
      adapters: this.getAvailableAdapters(),
      current: this.currentAdapter?.getCapabilities().name || null,
      ready: this.currentAdapter?.getStatus().ready || false
    };
  }
}
```

---

## Usage in Extension

### Background Service Worker

```javascript
// extension-w/background.js

import { LLMManager } from './llm/llm_manager.js';

let llmManager = null;

async function initializeLLM() {
  llmManager = new LLMManager();
  await llmManager.initialize();

  console.log('LLM Manager status:', llmManager.getStatus());
}

// Initialize on extension load
initializeLLM();

// Handle messages
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  if (request.type === 'llm_prompt') {
    (async () => {
      try {
        const response = await llmManager.prompt(request.messages, request.options);
        sendResponse({ success: true, response });
      } catch (error) {
        sendResponse({ success: false, error: error.message });
      }
    })();
    return true; // Keep channel open
  }

  if (request.type === 'llm_status') {
    sendResponse(llmManager.getStatus());
  }

  if (request.type === 'llm_set_adapter') {
    (async () => {
      try {
        await llmManager.setActiveAdapter(request.adapter);
        sendResponse({ success: true });
      } catch (error) {
        sendResponse({ success: false, error: error.message });
      }
    })();
    return true;
  }
});
```

### UI Integration

```javascript
// extension-w/ui/llm_config.js

async function loadLLMStatus() {
  const status = await chrome.runtime.sendMessage({ type: 'llm_status' });

  console.log('Available adapters:', status.adapters);
  console.log('Current:', status.current);
  console.log('Ready:', status.ready);

  // Populate UI
  const select = document.getElementById('llm-adapter');
  status.adapters.forEach(adapter => {
    const option = document.createElement('option');
    option.value = adapter.id;
    option.textContent = `${adapter.name} (${adapter.capabilities.type})`;
    select.appendChild(option);
  });
}

async function testLLM() {
  const response = await chrome.runtime.sendMessage({
    type: 'llm_prompt',
    messages: [
      { role: 'user', content: 'What is the capital of France?' }
    ]
  });

  console.log('LLM response:', response.response);
}
```

---

## Model Recommendations by Use Case

| Use Case | Framework | Model | Size | Speed |
|----------|-----------|-------|------|-------|
| **Quick tasks** | Chrome AI | Gemini Nano | N/A | Fastest |
| **Best quality** | WebLLM | Llama-3-8B-q4 | 4.5GB | Fast |
| **Balanced** | WebLLM | Phi-3-mini-q4 | 2.2GB | Fast |
| **Lightweight** | WebLLM | Gemma-2B-q4 | 1.5GB | Very Fast |
| **CPU fallback** | wllama | TinyLlama-Q4 | 669MB | Medium |
| **No WebGPU** | wllama | Phi-2-Q4 | 1.6GB | Medium |
| **Custom models** | ONNX | User ONNX | Varies | Medium |

---

## Performance Expectations

| Framework | First Load | Inference | Memory | Browser Support |
|-----------|------------|-----------|--------|-----------------|
| Chrome AI | <1s | 300-1000ms | ~100MB | Chrome 129+ |
| WebLLM | 10-60s (download) | 500-2000ms | 2-8GB | Chrome 113+, Edge 113+ |
| wllama | 5-30s (download) | 2-10s | 1-4GB | All modern browsers |
| ONNX Web | 5-20s (download) | 1-5s | 1-3GB | All modern browsers |

## Sources

- [WebLLM GitHub](https://github.com/mlc-ai/web-llm)
- [WebLLM Documentation](https://webllm.mlc.ai/)
- [wllama GitHub](https://github.com/ngxson/wllama)
- [llama-cpp-wasm](https://github.com/tangledgroup/llama-cpp-wasm)
- [ONNX Runtime Web](https://onnxruntime.ai/docs/tutorials/web/)
- [Mozilla: 3W for In-Browser AI](https://blog.mozilla.ai/3w-for-in-browser-ai-webllm-wasm-webworkers/)
- [Intel: Guide to In-Browser LLMs](https://www.intel.com/content/www/us/en/developer/articles/technical/web-developers-guide-to-in-browser-llms.html)
- [Picovoice: Cross-Browser LLM Inference](https://picovoice.ai/blog/cross-browser-local-llm-inference-using-webassembly/)
