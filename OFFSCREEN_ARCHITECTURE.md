# Offscreen Document Architecture for WebLLM/wllama

## Problem

Chrome extension service workers **cannot use dynamic `import()` statements**. This is a fundamental limitation of the ServiceWorkerGlobalScope per the HTML specification.

WebLLM and wllama libraries require dynamic imports from CDN:
```javascript
// This FAILS in service worker context:
const webllm = await import('https://esm.sh/@mlc-ai/web-llm@0.2.59');
```

**Error:**
```
TypeError: import() is disallowed on ServiceWorkerGlobalScope
```

## Previous Approach (Deferred Initialization)

We tried to defer the initialization:
1. Save config in service worker without initializing
2. Wait for first LLM call (from Ralph agent)
3. Call `ensureInitialized()` which calls `adapter.initialize()`
4. `adapter.initialize()` tries to do dynamic import

**Problem:** The Ralph agent runs in the background service worker, so when it calls `ensureInitialized()`, we're STILL in service worker context! The dynamic import still fails.

## Solution: Offscreen Document

Chrome provides **offscreen documents** specifically for this use case. Offscreen documents:
- Run in a **window context** (not service worker)
- Can use dynamic imports
- Are invisible to the user
- Communicate with service worker via messages

### Architecture

```
┌─────────────────┐
│  Wizard (tab)   │ Selects WebLLM
└────────┬────────┘
         │ llm_set_adapter
         v
┌─────────────────────────────┐
│  Background (service worker)│
│  OffscreenLLMProxy          │
└────────┬────────────────────┘
         │ Creates offscreen doc
         v
┌─────────────────────────────┐
│  Offscreen Document (window)│
│  - Handles dynamic imports  │
│  - Initializes WebLLM       │
│  - Processes prompts        │
└─────────────────────────────┘
```

### Message Flow

1. **Wizard** sends `llm_set_adapter` to background
2. **Background (OffscreenLLMProxy)**:
   - Detects WebLLM/wllama (needs offscreen)
   - Creates offscreen document
   - Forwards `offscreen_llm_set_adapter` to offscreen
3. **Offscreen** (window context):
   - Receives message
   - Creates LLMManager instance
   - Calls `setActiveAdapter()` with dynamic imports (works!)
4. **User** executes task in sidepanel
5. **Background (Ralph agent)**:
   - Calls `proxy.prompt(messages)`
   - Proxy forwards `offscreen_llm_prompt` to offscreen
6. **Offscreen**:
   - Calls `llmManager.prompt()` → WebLLM
   - Returns response via message
7. **Background (Ralph agent)**:
   - Receives response from offscreen
   - Continues task execution

## Files

### New Files

**`extension-w/offscreen.html`** (196 bytes)
- Minimal HTML file for offscreen document
- Just loads offscreen.js as module

**`extension-w/offscreen.js`** (2.3K)
- Creates LLMManager instance in window context
- Handles messages:
  - `offscreen_llm_set_adapter` - Initialize adapter
  - `offscreen_llm_prompt` - Send prompt to LLM
  - `offscreen_llm_status` - Get status
  - `offscreen_llm_list_adapters` - List adapters

### Modified Files

**`extension-w/background.js`** (+5K)
- Added `OffscreenLLMProxy` class:
  - Detects if adapter needs offscreen (WebLLM, wllama)
  - Creates offscreen document when needed
  - Forwards LLM operations to offscreen via messages
  - Handles non-offscreen adapters normally (Chrome AI, OpenAI, etc.)
- Added `ensureOffscreenDocument()` and `closeOffscreenDocument()`
- Replaced `llmManager = new LLMManager()` with `llmManager = new OffscreenLLMProxy()`

**`extension-w/manifest.json`**
- Added `"offscreen"` permission

**`extension-w/sidepanel.js`**
- Updated `updateStatus()` to handle pending status correctly
- Shows adapter name with ⏳ icon when initializing

## Benefits

1. **Solves the core problem:** Dynamic imports work in offscreen document
2. **Clean separation:** Service worker handles extension logic, offscreen handles LLM
3. **No hacks:** Uses official Chrome API (offscreen documents)
4. **Selective:** Only WebLLM/wllama use offscreen, others run in service worker
5. **Transparent:** Ralph agent doesn't need to change, proxy handles routing

## Testing

After loading the extension:

1. **Check service worker console** (`chrome://extensions` → service worker):
   ```
   [LLM Proxy] Using offscreen document for webllm
   [Oryn-W] Creating offscreen document for LLM operations
   [Oryn-W] Offscreen document created and ready
   ```

2. **Check offscreen console** (`chrome://inspect` → offscreen.html):
   ```
   [Offscreen] Starting LLM offscreen document
   [Offscreen] Setting adapter: webllm Gemma-2B-it-q4f16_1
   [WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
   [WebLLM] Download progress: 15.2%
   ```

3. **Execute a task** and watch offscreen process prompts:
   ```
   [Offscreen] Received message: offscreen_llm_prompt
   [WebLLM] Processing prompt...
   ```

## Adapters

| Adapter    | Uses Offscreen? | Reason                              |
|------------|-----------------|-------------------------------------|
| Chrome AI  | No              | Native browser API, no imports      |
| OpenAI     | No              | Simple fetch(), no dynamic imports  |
| Anthropic  | No              | Simple fetch(), no dynamic imports  |
| Gemini     | No              | Simple fetch(), no dynamic imports  |
| **WebLLM** | **Yes**         | Requires dynamic import from CDN    |
| **wllama** | **Yes**         | Requires dynamic import from CDN    |

## Future Improvements

1. **Lazy creation:** Only create offscreen when first needed (currently created on setActiveAdapter)
2. **Lifecycle management:** Close offscreen when switching to non-offscreen adapter
3. **Error recovery:** Better handling of offscreen crashes/restarts
4. **Performance:** Consider keeping offscreen alive vs recreating on each use

## References

- [Chrome Offscreen API](https://developer.chrome.com/docs/extensions/reference/offscreen/)
- [Service Worker Limitations](https://developer.mozilla.org/en-US/docs/Web/API/ServiceWorkerGlobalScope)
- [WebLLM Documentation](https://github.com/mlc-ai/web-llm)
