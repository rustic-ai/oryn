# Ralph Agent Integration for Extension-W

## Overview

This document outlines the architecture and implementation plan for integrating a Ralph-style agent with pluggable LLM support into the Oryn-W browser extension.

## Current vs Proposed Flow

### Current Flow
```
Input (OIL Command)
  → Background.js
  → WASM (oryn-core)
  → Action
  → Scanner.js
  → Result
```

### Proposed Flow
```
Input (Natural Language/Goal)
  → Agent Mode Detector
  → Ralph Agent (Local/Remote)
    → LLM Adapter (Chrome AI / Transformer.js / Remote API)
    → Generate OIL command
    → Execute via existing WASM pipeline
    → Observe results
    → Loop until goal complete or max iterations
  → Final Response with iteration history
```

## Architecture Components

### 1. Pluggable LLM System

#### LLM Adapter Interface
```javascript
class LLMAdapter {
  async initialize() {}
  async prompt(messages, options) {}
  async stream(messages, options) {}
  getCapabilities() { return { maxTokens, streaming, local }; }
  getStatus() { return { ready, error }; }
}
```

#### Supported LLM Adapters

| Adapter | Type | Backend | Latency | Cost | Privacy | Capability |
|---------|------|---------|---------|------|---------|------------|
| Chrome AI (Gemini Nano) | Local | Native API | 300-1000ms | Free | Private | Basic |
| **WebLLM** | Local WASM | WebGPU + WASM | 500-2000ms | Free | Private | High |
| **llama.cpp (wllama)** | Local WASM | WASM + SIMD | 2-5s | Free | Private | Medium-High |
| **ONNX Runtime Web** | Local WASM | WebGPU/WASM | 1-3s | Free | Private | Medium-High |
| Transformer.js | Local WASM | WASM | 2-5s | Free | Private | Medium |
| **picoLLM** | Local WASM | WASM + SIMD | 1-3s | Free | Private | Medium |
| OpenAI (GPT-4/3.5) | Remote API | Cloud | 500-2000ms | Paid | Shared | High |
| Claude (Sonnet/Haiku) | Remote API | Cloud | 500-2000ms | Paid | Shared | High |
| Gemini (Pro/Flash) | Remote API | Cloud | 400-1500ms | Paid | Shared | High |

#### LLM Manager
- Manages multiple adapters
- Selects active adapter based on user preference
- Handles fallbacks if primary adapter fails
- Stores API keys securely in `chrome.storage.sync`

### WASM-Based LLM Frameworks (Detailed Comparison)

#### 1. WebLLM (Recommended for High Performance)
**Repository:** [mlc-ai/web-llm](https://github.com/mlc-ai/web-llm)

**Highlights:**
- **Best performance:** Retains up to 80% of native LLM speed using WebGPU acceleration
- **OpenAI-compatible API:** Easy integration with familiar interface
- **Pre-compiled models:** Llama-3, Gemma, Phi-3, Mistral, etc. available on [HuggingFace mlc-ai](https://huggingface.co/mlc-ai)
- **WebGPU required:** Needs modern browser (Chrome 113+, Edge 113+)
- **Model size:** 2-8GB for quantized models (4-bit)

**Installation:**
```bash
npm install @mlc-ai/web-llm
```

**Example Usage:**
```javascript
import * as webllm from "@mlc-ai/web-llm";

const engine = await webllm.CreateMLCEngine("Llama-3-8B-Instruct-q4f16_1");
const reply = await engine.chat.completions.create({
  messages: [{ role: "user", content: "What is the capital of France?" }]
});
```

**Pros:**
- Fastest WASM LLM solution (WebGPU acceleration)
- Production-ready with good documentation
- Large model support (up to 70B quantized)

**Cons:**
- Requires WebGPU (no Safari support yet)
- Large model downloads (2-8GB)
- Memory intensive (4-8GB RAM)

#### 2. llama.cpp WASM Bindings

**Option A: wllama** (Recommended)
**Repository:** [ngxson/wllama](https://github.com/ngxson/wllama)

**Highlights:**
- Multi-threaded inference with SIMD support
- Supports GGUF format (most common for llama.cpp)
- Model splitting for large files
- Runs in Web Worker (non-blocking UI)

**Installation:**
```bash
npm install @wllama/wllama
```

**Example Usage:**
```javascript
import { Wllama } from '@wllama/wllama';

const wllama = new Wllama();
await wllama.loadModel({
  model: 'https://huggingface.co/user/model/resolve/main/model.gguf',
  useMultiThread: true
});

const result = await wllama.createCompletion({
  prompt: "What is the capital of France?",
  n_predict: 50
});
```

**Pros:**
- Works without WebGPU (pure WASM + SIMD)
- Compatible with all llama.cpp GGUF models
- Good for smaller models (<3GB)

**Cons:**
- Slower than WebGPU solutions (CPU-only)
- Limited to browser thread count
- Higher memory usage than native

**Option B: llama-cpp-wasm** (tangledgroup)
**Repository:** [tangledgroup/llama-cpp-wasm](https://github.com/tangledgroup/llama-cpp-wasm)

**Highlights:**
- Direct llama.cpp compilation to WASM
- Single-thread and multi-thread builds
- Lightweight for small models (TinyLlama, Phi-2)

**Usage:**
```html
<script src="llama-cpp-wasm.js"></script>
<script>
  const model = await LlamaCpp.load('model.gguf');
  const output = await model.generate('Hello');
</script>
```

**Pros:**
- Simple API for basic use cases
- Good for embedded/lightweight scenarios

**Cons:**
- Less actively maintained than wllama
- Limited documentation

#### 3. ONNX Runtime Web
**Repository:** [ONNX Runtime](https://onnxruntime.ai/docs/tutorials/web/)

**Highlights:**
- Multi-backend support: WebGPU, WebGL, WebNN, WASM
- Works with any ONNX-exported model
- Good for quantized models (INT8, FP16, INT4)
- Microsoft-backed, production-ready

**Installation:**
```bash
npm install onnxruntime-web
```

**Example Usage:**
```javascript
import * as ort from 'onnxruntime-web';

// Use WebGPU backend
ort.env.wasm.numThreads = 4;
const session = await ort.InferenceSession.create('model.onnx', {
  executionProviders: ['webgpu']
});

const feeds = { input: new ort.Tensor('float32', data, [1, 512]) };
const results = await session.run(feeds);
```

**Pros:**
- Versatile: supports multiple execution providers
- Good WebGPU performance (2-3x speedup with FP16)
- Works with quantized models
- Fallback support (WebGPU → WebGL → WASM)

**Cons:**
- Requires ONNX model format (conversion needed)
- More complex setup than WebLLM
- Lower-level API (manual tokenization)

#### 4. Transformer.js (Hugging Face)
**Repository:** [xenova/transformers.js](https://github.com/xenova/transformers.js)

**Highlights:**
- Easy one-line API for common tasks
- Auto-downloads models from Hugging Face
- Good for smaller models (<1GB)
- Supports vision, NLP, audio tasks

**Installation:**
```bash
npm install @xenova/transformers
```

**Example Usage:**
```javascript
import { pipeline } from '@xenova/transformers';

const generator = await pipeline('text-generation', 'Xenova/gpt2');
const output = await generator('Hello, I am', { max_length: 50 });
```

**Pros:**
- Simplest API (one-liner)
- Auto-downloads and caches models
- Good for smaller tasks

**Cons:**
- Limited to smaller models (<1GB recommended)
- Slower than WebLLM for LLMs
- Less suitable for large language models

#### 5. picoLLM
**Website:** [Picovoice picoLLM](https://picovoice.ai/blog/cross-browser-local-llm-inference-using-webassembly/)

**Highlights:**
- Cross-browser CPU-based inference
- WASM + SIMD optimizations
- Simple JavaScript SDK
- Commercial product (free tier available)

**Example Usage:**
```javascript
import { PicoLLM } from '@picovoice/picollm-web';

const llm = await PicoLLM.create('path/to/model.pv');
const response = await llm.generate('What is AI?');
```

**Pros:**
- Optimized for cross-browser compatibility
- Good CPU performance
- Simple API

**Cons:**
- Proprietary (requires license for production)
- Limited model selection
- Less community support

---

### Recommended WASM Framework Selection

**For Extension-W Ralph Agent, we recommend supporting multiple frameworks with priority:**

1. **Chrome AI** (if available) - Fastest, zero setup
2. **WebLLM** (if WebGPU available) - Best performance for serious LLMs
3. **wllama** (fallback) - Works everywhere, good for smaller models
4. **ONNX Runtime Web** (optional) - For custom/quantized models
5. **Remote APIs** - When local resources insufficient

**Implementation Strategy:**
```javascript
// Auto-detect best available adapter
async function selectBestAdapter() {
  // 1. Try Chrome AI first
  if (await ChromeAIAdapter.isAvailable()) {
    return new ChromeAIAdapter();
  }

  // 2. Try WebLLM if WebGPU available
  if (await WebLLMAdapter.isAvailable()) {
    return new WebLLMAdapter();
  }

  // 3. Fall back to wllama (works everywhere)
  return new WllamaAdapter();
}
```

### 2. Ralph Agent Implementation

#### Core Agent Loop
```
1. Retrieve similar trajectories (RAG)
2. Build few-shot prompt with examples + current state
3. Get LLM decision (thought + action)
4. Parse OIL command or completion signal
5. Execute OIL via existing pipeline
6. Observe results
7. Add to history
8. Repeat until task complete or max iterations
```

#### Agent Types

**RalphAgentLocal**
- Runs entirely in browser
- Uses LLM adapters for decision-making
- Stores trajectories in IndexedDB
- Executes OIL commands via background.js

**RalphAgentRemote**
- Proxies to remote Ralph service
- Streams decisions from server
- Executes OIL commands locally
- Useful for shared trajectory stores or more complex planning

### 3. Trajectory Store

**Storage:** IndexedDB in browser

**Schema:**
```javascript
{
  id: number,
  task: string,                    // "Buy a blue backpack"
  commands: [                      // Successful action sequence
    'type "blue backpack" into "search"',
    'click "Search"',
    'click "Add to Cart"',
    // ...
  ],
  success: boolean,
  timestamp: number,
  embedding: Array<number> | Array<string>  // For retrieval
}
```

**Retrieval Methods:**
1. **Keyword-based** (MVP) - Simple word overlap
2. **Embedding-based** - Cosine similarity (future)
3. **Hybrid** - Combine both approaches

**Operations:**
- `save(trajectory)` - Store successful trajectory
- `retrieve(task, k=3)` - Get k most similar examples
- `export()` / `import()` - Backup/restore trajectories
- `clear()` - Reset store

### 4. UI Updates

#### Mode Toggle
```
┌─────────────────────────────────┐
│  [OIL Mode] [Agent Mode*]       │
├─────────────────────────────────┤
│  Input: Buy a blue backpack     │
│  [Execute]                      │
└─────────────────────────────────┘
```

#### Agent Execution Display
```
Task: Buy a blue backpack

Step 1: Need to search for the product
  → type "blue backpack" into "search"
  ✓ Typed into element #7

Step 2: Submit the search
  → click "Search"
  ✓ Clicked element #12

Step 3: Found product, adding to cart
  → click "Add to Cart"
  ✓ Clicked element #23

✅ Task completed in 3 steps!
```

#### Configuration Modal
- Select LLM adapter (dropdown)
- Enter API keys (password fields)
- Set max iterations (slider: 1-20)
- Trajectory management (export/import/clear)
- Test LLM connection (button)

### 5. Prompt Engineering

#### Ralph Few-Shot Prompt Template
```
You are a web automation agent using OIL (Oryn Intent Language).

SUCCESSFUL EXAMPLES:
{examples from trajectory store}

CURRENT TASK: {user task}

CURRENT PAGE:
URL: {page.url}
Title: {page.title}
Elements:
{formatted element list}

PREVIOUS ACTIONS:
{history of actions in this session}

Based on the examples and current state, what should be the next action?

Respond in this format:
Thought: <your reasoning about what to do next>
Action: <OIL command to execute>

Or if the task is complete:
Thought: <reasoning why task is complete>
Status: Task complete
```

#### Response Parsing
```javascript
parseDecision(llmOutput) {
  // Extract thought and action
  const thoughtMatch = llmOutput.match(/Thought: (.*)/);
  const actionMatch = llmOutput.match(/Action: (.*)/);
  const statusMatch = llmOutput.match(/Status: (.*complete.*)/i);

  if (statusMatch) {
    return { type: 'complete', message: thoughtMatch?.[1] };
  }

  return {
    type: 'action',
    thought: thoughtMatch?.[1],
    command: actionMatch?.[1]
  };
}
```

## Implementation Plan

### Phase 1: LLM Adapter System (2-3 days)
**Goal:** Build pluggable LLM foundation

Files to create:
- `extension-w/llm/llm_adapter.js` - Base interface
- `extension-w/llm/llm_manager.js` - Adapter manager
- `extension-w/llm/chrome_ai_adapter.js` - Gemini Nano support
- `extension-w/ui/llm_config.html` - Configuration UI

Tasks:
1. Define `LLMAdapter` interface
2. Implement `ChromeAIAdapter` with error handling
3. Create `LLMManager` for adapter selection
4. Build configuration UI
5. Test with simple prompts from sidepanel

**Success Criteria:**
- Can send prompt to Chrome AI and receive response
- Configuration persists in chrome.storage
- UI shows LLM status (ready/error)

### Phase 2: Trajectory Store (1-2 days)
**Goal:** Enable RAG-style few-shot learning

Files to create:
- `extension-w/agent/trajectory_store.js` - IndexedDB wrapper

Tasks:
1. Implement IndexedDB schema and operations
2. Add keyword-based retrieval algorithm
3. Create seed trajectories for common tasks
4. Add export/import functionality
5. Add trajectory viewer in UI

**Success Criteria:**
- Can save and retrieve trajectories
- Retrieval returns relevant examples
- Can export/import trajectory database

**Seed Trajectories:**
```javascript
[
  {
    task: "Search for laptops",
    commands: [
      'type "laptops" into "search"',
      'click "Search"'
    ]
  },
  {
    task: "Login to account",
    commands: [
      'type "user@example.com" into "email"',
      'type "password123" into "password"',
      'click "Login"'
    ]
  },
  {
    task: "Add item to cart",
    commands: [
      'click "Add to Cart"',
      'click "View Cart"'
    ]
  },
  // ... 10-20 more examples
]
```

### Phase 3: Ralph Agent Core (3-4 days)
**Goal:** Implement agent decision loop

Files to create:
- `extension-w/agent/ralph_agent.js` - Core agent logic
- `extension-w/agent/prompts.js` - Prompt templates

Tasks:
1. Implement `RalphAgentLocal` class
2. Build few-shot prompt builder
3. Add decision parsing logic
4. Integrate with existing OIL execution pipeline
5. Add iteration tracking and history
6. Implement completion detection
7. Add error recovery (retry on parse errors)

**Success Criteria:**
- Agent can complete simple tasks (e.g., "Search for shoes")
- Generates valid OIL commands
- Stops on task completion or max iterations
- Returns full execution history

### Phase 4: UI Integration (2-3 days)
**Goal:** Make agent accessible from sidepanel

Files to modify:
- `extension-w/sidepanel.html` - Add mode toggle
- `extension-w/sidepanel.js` - Add agent execution flow
- `extension-w/background.js` - Add agent message handler

Tasks:
1. Add OIL/Agent mode toggle
2. Update input placeholder based on mode
3. Implement agent execution UI flow
4. Display agent iterations with thoughts
5. Show OIL commands generated
6. Add progress indicators
7. Style agent messages differently from OIL

**Success Criteria:**
- Can toggle between OIL and Agent mode
- Agent mode accepts natural language
- Shows step-by-step execution
- Displays final result clearly

### Phase 5: Remote LLM Support (2-3 days)
**Goal:** Add OpenAI, Claude, Gemini adapters

Files to create:
- `extension-w/llm/openai_adapter.js`
- `extension-w/llm/claude_adapter.js`
- `extension-w/llm/gemini_adapter.js`

Tasks:
1. Implement OpenAI API adapter
2. Implement Claude API adapter
3. Implement Gemini API adapter
4. Add API key management UI
5. Add streaming support for faster responses
6. Test with all three providers

**Success Criteria:**
- Can authenticate with each provider
- Generates correct API requests
- Handles rate limits and errors gracefully
- Streaming works (optional)

### Phase 6: WASM LLM Support (4-6 days)
**Goal:** Add local WASM model support with multiple frameworks

Files to create:
- `extension-w/llm/webllm_adapter.js` - WebLLM (WebGPU)
- `extension-w/llm/wllama_adapter.js` - llama.cpp WASM
- `extension-w/llm/onnxweb_adapter.js` - ONNX Runtime Web
- `extension-w/llm/transformerjs_adapter.js` - Transformer.js (legacy)
- `extension-w/models/` - Model cache directory
- `extension-w/ui/model_manager.html` - Model download UI

Tasks:
1. **WebLLM Integration** (Priority 1)
   - Install `@mlc-ai/web-llm`
   - Implement adapter with OpenAI-compatible API
   - Add model selection UI (Llama-3, Gemma, Phi-3)
   - Download progress tracking
   - Test with 4-bit quantized models

2. **wllama Integration** (Priority 2)
   - Install `@wllama/wllama`
   - Implement adapter for GGUF models
   - Multi-threaded inference setup
   - Model splitting for large files
   - Test with Llama-2-7B GGUF

3. **ONNX Runtime Web** (Priority 3)
   - Install `onnxruntime-web`
   - Implement multi-backend adapter (WebGPU/WASM)
   - Add quantized model support
   - Custom tokenizer integration
   - Test with exported ONNX models

4. **Transformer.js** (Optional)
   - Keep existing implementation
   - Use for lightweight tasks only

5. **Model Management UI**
   - Model catalog with recommendations
   - Download/cache management
   - Size and performance estimates
   - One-click model installation

**Success Criteria:**
- All three frameworks work independently
- Automatic selection based on browser capabilities
- Models download with clear progress
- Inference completes in <5s for WebLLM, <10s for others
- Memory usage <4GB
- Can switch between frameworks dynamically

**Recommended Models by Framework:**

| Framework | Model | Size | Speed | Use Case |
|-----------|-------|------|-------|----------|
| WebLLM | Llama-3-8B-Instruct-q4f16_1 | 4.5GB | Fast (WebGPU) | Best quality |
| WebLLM | Phi-3-mini-4k-instruct-q4f16_1 | 2.2GB | Very Fast | Balanced |
| WebLLM | Gemma-2B-it-q4f16_1 | 1.5GB | Fastest | Quick tasks |
| wllama | TinyLlama-1.1B-Chat-v1.0.Q4 | 669MB | Medium | Fallback |
| wllama | Phi-2-Q4_K_M | 1.6GB | Medium | Good balance |
| ONNX | Custom quantized models | Varies | Medium | Specialized |

**Challenges:**
- Large model files (1.5-8GB)
- Browser memory limits
- WebGPU browser support
- Model download time

**Solutions:**
- Start with smaller models (Gemma-2B, Phi-3-mini)
- Progressive loading with chunking
- IndexedDB for model caching
- Clear size/performance tradeoffs in UI
- Fallback chain: WebLLM → wllama → Remote API

### Phase 7: Remote Agent Support (2 days)
**Goal:** Support server-side Ralph execution

Files to create:
- `extension-w/agent/ralph_agent_remote.js`

Tasks:
1. Implement remote agent proxy
2. Add server endpoint configuration
3. Handle streaming responses
4. Test with mock server
5. Document API contract

**Success Criteria:**
- Can connect to remote Ralph service
- Executes OIL commands locally
- Receives agent decisions from server
- Handles connection errors

**Remote Agent API:**
```
POST /agent/execute
{
  "task": "Buy a blue backpack",
  "observation": { /* scan result */ },
  "history": [ /* previous actions */ ]
}

Response (streaming):
{
  "thought": "Need to search first",
  "command": "type \"blue backpack\" into \"search\""
}
```

### Phase 8: Polish & Testing (2-3 days)
**Goal:** Production-ready quality

Tasks:
1. E2E tests with all LLM adapters
2. Performance benchmarking
3. Error handling improvements
4. Loading states and spinners
5. Documentation (README, examples)
6. Create demo video
7. Seed 20+ example trajectories

**Test Cases:**
- Simple search task
- Multi-step e-commerce flow
- Form filling
- Login flow
- Error recovery (invalid command)
- Max iterations reached
- LLM timeout handling

## File Structure

```
extension-w/
├── manifest.json                   # Updated with new permissions
├── background.js                   # Updated with agent handlers
├── sidepanel.html                  # Updated with mode toggle
├── sidepanel.js                    # Updated with agent execution
├── llm/
│   ├── llm_adapter.js             # Base interface
│   ├── llm_manager.js             # Adapter manager
│   ├── chrome_ai_adapter.js       # Chrome AI / Gemini Nano
│   ├── transformerjs_adapter.js   # Transformer.js WASM
│   ├── openai_adapter.js          # OpenAI API
│   ├── claude_adapter.js          # Anthropic API
│   └── gemini_adapter.js          # Google AI API
├── agent/
│   ├── ralph_agent.js             # Local Ralph agent
│   ├── ralph_agent_remote.js      # Remote Ralph proxy
│   ├── trajectory_store.js        # IndexedDB storage
│   ├── prompts.js                 # Prompt templates
│   └── seed_trajectories.js       # Example trajectories
├── ui/
│   ├── llm_config.html            # LLM configuration modal
│   ├── llm_config.js              # Configuration logic
│   ├── trajectory_viewer.html     # Trajectory browser
│   └── styles.css                 # Updated styles
├── models/                         # Downloaded WASM models
│   ├── gemma-2b/
│   └── phi-3.5/
└── vendor/
    └── transformerjs/              # Transformer.js library
```

## Manifest Updates

```json
{
  "permissions": [
    "storage",              // For API keys and config
    "activeTab",
    "scripting",
    "tabs",
    "sidePanel"
  ],
  "host_permissions": [
    "<all_urls>",           // For page access
    "https://api.openai.com/*",
    "https://api.anthropic.com/*",
    "https://generativelanguage.googleapis.com/*"
  ],
  "web_accessible_resources": [
    {
      "resources": ["models/*"],
      "matches": ["<all_urls>"]
    }
  ]
}
```

## Configuration Storage

```javascript
// chrome.storage.sync schema
{
  llmAdapter: 'chrome-ai',           // Selected adapter
  apiKeys: {
    openai: 'sk-...',
    claude: 'sk-ant-...',
    gemini: 'AI...'
  },
  agentConfig: {
    maxIterations: 10,
    temperature: 0.7,
    showThoughts: true
  },
  trajectorySettings: {
    retrievalCount: 3,
    autoSave: true
  }
}
```

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| LLM latency (Chrome AI) | <1s | Per decision |
| LLM latency (Remote) | <2s | Network-dependent |
| Agent iteration | <3s | LLM + execution |
| Total task completion | <30s | For 10 iterations |
| Memory overhead | <100MB | Excluding WASM models |
| WASM model size | <500MB | Quantized models |

## Risk Mitigation

### Risk 1: Chrome AI Availability
**Mitigation:** Graceful fallback to remote LLMs with clear user messaging

### Risk 2: Slow Local Models
**Mitigation:**
- Use smaller quantized models
- Offload to web workers
- Show clear progress indicators
- Default to Chrome AI or remote

### Risk 3: Poor LLM Decisions
**Mitigation:**
- Curate high-quality seed trajectories
- Improve prompt engineering
- Add validation before execution
- Allow user to override/correct

### Risk 4: API Cost Concerns
**Mitigation:**
- Default to free local models
- Show cost estimates
- Add usage tracking
- Cache common patterns

## Future Enhancements

1. **Advanced Trajectory Retrieval**
   - Semantic embeddings (not just keywords)
   - Hybrid search (dense + sparse)
   - User feedback to improve ranking

2. **Multi-Modal Support**
   - Include screenshots in prompts
   - Vision models for better understanding
   - OCR for complex UIs

3. **Collaborative Learning**
   - Shared trajectory repository
   - Community-contributed examples
   - Federated learning

4. **Advanced Planning**
   - Task decomposition
   - Parallel action execution
   - Rollback on failure

5. **Integration with Other Agents**
   - ReflexionAgent for self-improvement
   - PlanActAgent for complex tasks
   - Ensemble of agents

## Questions for Discussion

1. **Default LLM:** Should we default to Chrome AI (fastest, local) or remote (most capable)?

2. **Trajectory Seeding:** How many seed trajectories do we need? Should we include domain-specific ones (e-commerce, forms, etc.)?

3. **Max Iterations:** What's a reasonable default? 10 seems safe but might be limiting.

4. **Error Recovery:** Should we retry on parse errors? How many times?

5. **Privacy:** Should we warn users before sending page content to remote LLMs?

6. **Cost Tracking:** Should we track API costs and show estimates?

7. **Model Downloading:** Should we include WASM models in the extension or download on-demand?

8. **Remote Agent:** Is there a Ralph service API we can use, or do we need to build one?

## References

- Ralph Agent Paper: [Link if available]
- IntentGym Ralph Implementation: `intentgym/src/intentgym/agents/ralph.py`
- Chrome AI Documentation: https://developer.chrome.com/docs/ai/built-in
- Transformer.js: https://huggingface.co/docs/transformers.js
- OIL Specification: `grammar/OIL.md`

## Conclusion

This integration will transform extension-w from a command executor into an intelligent agent that can understand natural language goals and autonomously complete multi-step tasks. The pluggable LLM architecture ensures flexibility, privacy, and cost control while maintaining the fast, local-first philosophy of oryn-w.

Total estimated effort: **18-26 days** for full implementation across all phases.

MVP (Phases 1-4): **8-12 days** for basic agent with Chrome AI support.
