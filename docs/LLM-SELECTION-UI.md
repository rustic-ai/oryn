# LLM Selection and Switching UI

This document describes the UI components for selecting and switching between different LLM providers in Extension-W.

## Components

### 1. Full Configuration Page (`ui/llm_selector.html`)

A dedicated configuration page that provides:
- **Adapter Selection**: Visual cards for each available LLM adapter
- **Status Indicators**: Real-time availability status for each adapter
- **Model Selection**: Dropdown to choose specific models (for adapters that support multiple models)
- **API Key Management**: Secure input fields for remote API keys
- **Agent Settings**: Sliders for max iterations, temperature, and retrieval count
- **Test Interface**: Test prompt functionality to verify LLM is working
- **Persistence**: Save/load configuration from chrome.storage

### 2. Compact Status Widget (`ui/llm_status_widget.html`)

An embeddable widget for the sidepanel that shows:
- **Current Adapter**: Active LLM with icon and name
- **Status Indicator**: Green/yellow/red dot showing readiness
- **Quick Switcher**: Dropdown to quickly switch between available adapters
- **Configuration Link**: Button to open full configuration page

## File Structure

```
extension-w/
├── ui/
│   ├── llm_selector.html        # Full configuration page
│   ├── llm_selector.js          # Configuration page logic
│   ├── llm_status_widget.html   # Embeddable status widget
│   └── styles.css               # Shared styles (optional)
└── manifest.json                # Updated to include new pages
```

## Integration Guide

### Step 1: Update Manifest

```json
{
  "web_accessible_resources": [
    {
      "resources": [
        "ui/llm_selector.html",
        "ui/llm_status_widget.html"
      ],
      "matches": ["<all_urls>"]
    }
  ]
}
```

### Step 2: Integrate Widget into Sidepanel

Add to `sidepanel.html` after the header:

```html
<!DOCTYPE html>
<html>
<head>
    <title>Oryn-W Sidepanel</title>
    <!-- Existing styles -->
</head>
<body>
    <div class="container">
        <h1>Oryn-W Control Panel</h1>

        <!-- LLM Status Widget -->
        <div id="llm-widget-container"></div>

        <!-- Rest of sidepanel content -->
        <!-- ... -->
    </div>

    <script>
        // Load LLM widget
        fetch(chrome.runtime.getURL('ui/llm_status_widget.html'))
            .then(response => response.text())
            .then(html => {
                document.getElementById('llm-widget-container').innerHTML = html;
            });
    </script>

    <!-- Existing scripts -->
</body>
</html>
```

### Step 3: Update Background.js

Add message handlers for LLM management:

```javascript
// extension-w/background.js

import { LLMManager } from './llm/llm_manager.js';

let llmManager = null;

// Initialize LLM Manager
async function initializeLLM() {
    llmManager = new LLMManager();
    await llmManager.initialize();
    console.log('LLM Manager initialized:', llmManager.getStatus());
}

// Initialize on extension load
initializeLLM();

// Message handlers
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    // Get LLM status
    if (request.type === 'llm_status') {
        const status = llmManager.getStatus();
        sendResponse(status);
        return;
    }

    // Set active adapter
    if (request.type === 'llm_set_adapter') {
        (async () => {
            try {
                await llmManager.setActiveAdapter(request.adapter);

                // Initialize with model and API key if provided
                if (request.model || request.apiKey) {
                    const adapter = llmManager.getCurrentAdapter();
                    await adapter.initialize(request.model, {
                        apiKey: request.apiKey,
                        ...request.settings
                    });
                }

                // Save to storage
                await chrome.storage.sync.set({
                    llmConfig: {
                        selectedAdapter: request.adapter,
                        selectedModel: request.model,
                        apiKeys: { [request.adapter]: request.apiKey },
                        agentSettings: request.settings
                    }
                });

                sendResponse({ success: true });
            } catch (error) {
                console.error('Failed to set adapter:', error);
                sendResponse({ success: false, error: error.message });
            }
        })();
        return true; // Keep channel open for async response
    }

    // Send prompt to LLM
    if (request.type === 'llm_prompt') {
        (async () => {
            try {
                const response = await llmManager.prompt(
                    request.messages,
                    request.options || {}
                );
                sendResponse({ success: true, response });
            } catch (error) {
                console.error('LLM prompt failed:', error);
                sendResponse({ success: false, error: error.message });
            }
        })();
        return true;
    }

    // Existing handlers...
});

// Listen for storage changes to update adapter
chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'sync' && changes.llmConfig) {
        const config = changes.llmConfig.newValue;
        if (config && config.selectedAdapter) {
            llmManager.setActiveAdapter(config.selectedAdapter).catch(console.error);
        }
    }
});
```

## UI Screenshots (Mockup)

### Full Configuration Page

```
┌─────────────────────────────────────────────────────────────┐
│ 🤖 LLM Configuration                                        │
│ Select and configure your preferred language model          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│ Select LLM Provider                                          │
│ ┌──────────────────────────────────────────────────────┐   │
│ │ ⚡ Chrome AI (Gemini Nano)              ✓ Ready      │   │
│ │ Built-in browser AI. Fast, private, and free.        │   │
│ │ [Speed: 300-1000ms] [Privacy: Private] [Cost: Free] │   │
│ └──────────────────────────────────────────────────────┘   │
│                                                              │
│ ┌──────────────────────────────────────────────────────┐   │
│ │ 🚀 WebLLM                               ✓ Ready      │   │
│ │ GPU-accelerated local LLMs. High quality responses.  │   │
│ │ [Speed: 500-2000ms] [Quality: High] [WebGPU: Req]   │   │
│ │ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄ │   │
│ │ Select Model: [Phi-3 Mini (2.2GB)     ▼]            │   │
│ │ Size: 2.2GB • Quality: Good                          │   │
│ └──────────────────────────────────────────────────────┘   │
│                                                              │
│ ┌──────────────────────────────────────────────────────┐   │
│ │ 🤖 OpenAI                               ✗ Unavail    │   │
│ │ GPT-4 and GPT-3.5. Most capable but requires key.   │   │
│ │ [Speed: 500-2000ms] [Quality: Excellent] [Cost: $]  │   │
│ └──────────────────────────────────────────────────────┘   │
│                                                              │
├─────────────────────────────────────────────────────────────┤
│ Agent Settings                                               │
│ Max Iterations: [━━━━━━━━━━○━━━━━━━━━] 10                  │
│ Temperature:    [━━━━━━━━━━━━━○━━━━━] 0.7                  │
│ Retrieval Count:[━━━━○━━━━━━━━━━━━━━] 3                    │
├─────────────────────────────────────────────────────────────┤
│ Test LLM Connection                                          │
│ ┌──────────────────────────────────────────────────────┐   │
│ │ What is the capital of France?                       │   │
│ └──────────────────────────────────────────────────────┘   │
│ [Test Prompt]                                                │
│                                                              │
│ ✓ Success (593ms)                                            │
│ The capital of France is Paris.                              │
├─────────────────────────────────────────────────────────────┤
│ [Reset to Defaults]                  [Save Configuration]   │
└─────────────────────────────────────────────────────────────┘
```

### Compact Status Widget (in Sidepanel)

```
┌────────────────────────────────────────┐
│ LLM Provider               ● Ready     │
│ ⚡ Chrome AI                           │
│                                        │
│ [Switch LLM ▼]      [Configure]       │
└────────────────────────────────────────┘

When "Switch LLM" clicked:
┌────────────────────────────────────────┐
│ LLM Provider               ● Ready     │
│ ⚡ Chrome AI                           │
│                                        │
│ [Switch LLM ▲]      [Configure]       │
│ ┌────────────────────────────────────┐│
│ │ ⚡ Chrome AI              ✓        ││
│ │ Available                          ││
│ ├────────────────────────────────────┤│
│ │ 🚀 WebLLM                          ││
│ │ Available                          ││
│ ├────────────────────────────────────┤│
│ │ 🦙 llama.cpp                       ││
│ │ Available                          ││
│ ├────────────────────────────────────┤│
│ │ 🤖 OpenAI                          ││
│ │ Not available                      ││
│ ├────────────────────────────────────┤│
│ │ ⚙️ Full Configuration              ││
│ └────────────────────────────────────┘│
└────────────────────────────────────────┘
```

## User Workflows

### Workflow 1: First-Time Setup

1. User installs extension
2. Opens sidepanel
3. Sees "No LLM Selected" in widget
4. Clicks "Configure"
5. Opens full configuration page
6. Sees available adapters (Chrome AI, WebLLM if WebGPU available)
7. Selects Chrome AI (auto-selected if available)
8. Clicks "Save Configuration"
9. Returns to sidepanel, sees "Chrome AI - Ready"

### Workflow 2: Switching Between Local Models

1. User has Chrome AI configured
2. Wants to try WebLLM for better quality
3. Clicks "Switch LLM" in widget
4. Sees dropdown with available options
5. Clicks "WebLLM"
6. Extension loads WebLLM adapter
7. Widget shows "WebLLM - Ready" (after model download)

### Workflow 3: Adding Remote API

1. User wants to use GPT-4
2. Opens "Configure" from widget
3. Scrolls to OpenAI card
4. Clicks OpenAI card
5. Enters API key in revealed input field
6. Selects GPT-4 model from dropdown
7. Clicks "Test Prompt" to verify
8. Sees successful response
9. Clicks "Save Configuration"
10. Returns to sidepanel, can now use GPT-4

### Workflow 4: Testing Different Models

1. User has multiple adapters configured
2. Working on a complex task
3. Clicks "Switch LLM" in sidepanel
4. Tries Chrome AI - fast but basic
5. Switches to WebLLM with Llama-3 - better quality
6. Switches to GPT-4 for critical task - best results
7. All switches take <2 seconds

## Configuration Storage Schema

```javascript
// chrome.storage.sync
{
  llmConfig: {
    selectedAdapter: 'webllm',              // Currently active adapter
    selectedModel: 'Phi-3-mini-4k-instruct-q4f16_1', // Model ID
    apiKeys: {
      openai: 'sk-...',
      claude: 'sk-ant-...',
      gemini: 'AI...'
    },
    agentSettings: {
      maxIterations: 10,
      temperature: 0.7,
      retrievalCount: 3
    }
  }
}
```

## API Reference

### Message Types

#### `llm_status`
Get current LLM status and available adapters.

**Request:**
```javascript
chrome.runtime.sendMessage({ type: 'llm_status' })
```

**Response:**
```javascript
{
  adapters: [
    {
      id: 'chrome-ai',
      name: 'Chrome AI',
      available: true,
      capabilities: { /* ... */ }
    },
    // ...
  ],
  current: 'chrome-ai',           // Currently active adapter
  ready: true,                    // Ready to accept prompts
  model: 'gemini-nano'            // Current model (if applicable)
}
```

#### `llm_set_adapter`
Switch to a different LLM adapter.

**Request:**
```javascript
chrome.runtime.sendMessage({
  type: 'llm_set_adapter',
  adapter: 'webllm',
  model: 'Phi-3-mini-4k-instruct-q4f16_1',  // Optional
  apiKey: 'sk-...',                         // Optional (for remote)
  settings: {                               // Optional
    temperature: 0.7,
    maxTokens: 512
  }
})
```

**Response:**
```javascript
{ success: true }
// or
{ success: false, error: 'Error message' }
```

#### `llm_prompt`
Send a prompt to the current LLM.

**Request:**
```javascript
chrome.runtime.sendMessage({
  type: 'llm_prompt',
  messages: [
    { role: 'user', content: 'What is the capital of France?' }
  ],
  options: {
    temperature: 0.7,
    maxTokens: 100
  }
})
```

**Response:**
```javascript
{
  success: true,
  response: 'The capital of France is Paris.'
}
// or
{
  success: false,
  error: 'Error message'
}
```

## Styling Guidelines

### Colors

- **Primary**: `#667eea` (Purple gradient)
- **Success**: `#4ade80` (Green)
- **Warning**: `#fbbf24` (Yellow)
- **Error**: `#f87171` (Red)
- **Background**: `#f5f5f5` (Light gray)
- **Card**: `#ffffff` (White)

### Typography

- **Headers**: System fonts (-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto)
- **Monospace**: For API keys and technical values
- **Font Sizes**:
  - Page title: 24px
  - Section title: 18px
  - Card title: 16px
  - Body text: 14px
  - Small text: 13px
  - Hints: 11px

### Icons

Each adapter has an emoji icon for quick visual identification:
- ⚡ Chrome AI
- 🚀 WebLLM
- 🦙 llama.cpp (wllama)
- ⚙️ ONNX Runtime
- 🤖 OpenAI
- 🧠 Claude
- ✨ Gemini

## Accessibility

- **Keyboard Navigation**: All interactive elements are keyboard accessible
- **ARIA Labels**: Proper labels for screen readers
- **Color Contrast**: WCAG AA compliant contrast ratios
- **Focus Indicators**: Clear focus states for all controls
- **Status Messages**: Announced via ARIA live regions

## Future Enhancements

1. **Model Download Progress**: Show detailed progress for large model downloads
2. **Cost Tracking**: Track API usage and estimated costs for remote models
3. **Performance Metrics**: Show average response times and quality ratings
4. **Favorites**: Quick access to frequently used adapter/model combinations
5. **Profiles**: Save different configurations for different use cases
6. **Batch Testing**: Test same prompt across multiple adapters to compare
7. **Auto-Fallback**: Automatically switch to backup adapter if primary fails

## Troubleshooting

### Widget Not Showing
- Verify `llm_status_widget.html` is loaded correctly
- Check browser console for errors
- Ensure manifest includes web_accessible_resources

### Adapter Unavailable
- Chrome AI: Check Chrome version (129+) and feature flags
- WebLLM: Verify WebGPU support in browser
- Remote APIs: Check API key validity and network connection

### Configuration Not Saving
- Check chrome.storage.sync permissions in manifest
- Verify storage quota not exceeded
- Look for errors in background service worker console

## Testing

### Manual Testing Checklist

- [ ] All adapters display with correct status
- [ ] Can select each available adapter
- [ ] Model dropdown populates correctly
- [ ] API key input saves and loads
- [ ] Sliders update values correctly
- [ ] Test prompt works for each adapter
- [ ] Configuration persists after browser restart
- [ ] Quick switcher shows all options
- [ ] Widget updates when configuration changes
- [ ] Opens full config page from widget

### Automated Tests

```javascript
// Example test
describe('LLM Selector UI', () => {
  it('should display all available adapters', async () => {
    const adapters = await getAvailableAdapters();
    expect(adapters.length).toBeGreaterThan(0);
  });

  it('should save configuration', async () => {
    await selectAdapter('chrome-ai');
    await saveConfiguration();
    const config = await loadConfiguration();
    expect(config.selectedAdapter).toBe('chrome-ai');
  });
});
```

## Conclusion

This UI system provides users with:
- **Flexibility**: Easy switching between multiple LLM providers
- **Transparency**: Clear status and capability information
- **Control**: Fine-grained configuration options
- **Convenience**: Quick switcher for common operations
- **Safety**: Secure API key storage and validation

The design prioritizes user experience while maintaining technical robustness and extensibility for future LLM providers.
