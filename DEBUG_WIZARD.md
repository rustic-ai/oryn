# Extension-W First-Run Wizard - Debug Guide

## Current Status

Extension is loading successfully with enhanced logging.

## How to Test and Debug

### 1. Launch Browser with Extension

```bash
cd /home/rohit/work/dragonscale/oryn
./scripts/test-extension-w.sh
```

This will:
- Launch Chromium with the extension-w loaded
- Auto-open the first-run wizard
- Capture all console logs to `extension-w-test.log`

### 2. Open DevTools

**For Service Worker (Background Script):**
1. Go to `chrome://extensions`
2. Find "Oryn-W" extension
3. Click the blue "service worker" link
4. DevTools will open showing background logs

**For Wizard Page:**
1. The wizard opens automatically in a new tab
2. Press `F12` to open DevTools on that tab
3. Go to Console tab

### 3. What to Look For

#### In Service Worker Console:

Look for these log sequences:

**Normal flow:**
```
[LLM Manager] Setting active adapter: webllm model: Gemma-2B-it-q4f16_1
[LLM Manager] requiresDynamicImport: true inServiceWorker: true
[LLM Manager] Deferring initialization for webllm (service worker context)
[LLM Manager] Pending config: {name: 'webllm', model: 'Gemma-2B-it-q4f16_1', config: {}}
[LLM Manager] Adapter configuration saved, will initialize on first use
```

**Error indicators:**
```
[LLM Manager] Failed to...
[ERROR] ...
Error: ...
```

#### In Wizard Console:

**Normal flow:**
```
[Wizard] Starting download for adapter: webllm model: Gemma-2B-it-q4f16_1
[Wizard] Sending llm_set_adapter message to background...
[Wizard] llm_set_adapter response: {success: true, ...}
[Wizard] Starting progress polling...
```

**Error indicators:**
```
[Wizard] Download failed...
[Wizard] Background returned error: ...
```

### 4. Test the Wizard Flow

1. **Hardware Check (Step 1):**
   - Should complete automatically
   - Shows WebGPU, Chrome AI availability
   - Click "Next"

2. **Choose Model (Step 2):**
   - Select "WebLLM (GPU-Accelerated)"
   - Model dropdown should appear
   - Select a model (e.g., "Gemma-2B-it-q4f16_1")
   - Button should say "Download & Continue"
   - Click it

3. **Watch the Logs:**
   - Service worker should log: "Deferring initialization"
   - Wizard should advance to Step 3
   - Step 3 should show orange note: "The model will download when you first use Oryn"

4. **Test Actual Download:**
   - Click "Open Oryn" to open sidepanel
   - Type a test prompt
   - Watch service worker console for:
     ```
     [LLM Manager] ensureInitialized called
     [LLM Manager] Performing deferred initialization for webllm
     ```

### 5. Known Issues to Check

1. **Emoji/Glyph Issues:**
   - If you see `∂¥§` instead of emojis, it's an encoding issue
   - Check if HTML file has `<meta charset="UTF-8">`

2. **Configuration Not Persisting:**
   - Check service worker console for:
     ```
     [LLM Manager] Loading configuration from storage...
     [LLM Manager] Storage result: ...
     ```
   - Use DevTools → Application → Storage → chrome.storage to inspect

3. **Download Not Starting:**
   - Check if `ensureInitialized()` is being called
   - Look for "Performing deferred initialization" log
   - Check for import errors

### 6. Collect Debug Information

After testing, collect:

1. **Service Worker Logs:**
   ```bash
   grep "LLM Manager" extension-w-test.log > llm-logs.txt
   grep "ERROR\|error" extension-w-test.log > errors.txt
   ```

2. **Wizard Logs:**
   - Copy/paste from browser DevTools console
   - Or screenshot the console

3. **Configuration State:**
   - In DevTools → Application → Storage → chrome.storage.sync
   - Look for `llmConfig`
   - Screenshot or copy the JSON

### 7. Quick Log Analysis

```bash
# View all LLM-related logs
grep -E "(LLM Manager|Wizard|WebLLM)" extension-w-test.log

# View errors only
grep -i "error\|fail\|exception" extension-w-test.log | grep -v "DEPRECATED_ENDPOINT"

# View configuration flow
grep -E "(setActiveAdapter|loadConfig|saveConfig)" extension-w-test.log

# View initialization flow
grep -E "(ensureInitialized|Deferred initialization|Pending config)" extension-w-test.log
```

## Expected vs Actual Behavior

### Expected (With Deferred Init):

1. User selects WebLLM in wizard → Config saved, no download
2. Wizard completes → Shows note about download on first use
3. User opens sidepanel → Still no download (just UI loads)
4. User sends first prompt → Download starts NOW
5. Progress shows in sidepanel → Download completes
6. Prompt gets answered

### To Verify:

- [ ] Wizard completes without errors
- [ ] Step 3 shows correct adapter and model name
- [ ] Service worker logs show "Deferring initialization"
- [ ] Configuration persists (check chrome.storage)
- [ ] Sidepanel opens successfully
- [ ] First prompt triggers deferred initialization
- [ ] Download progress appears
- [ ] Model downloads and responds

## Troubleshooting

If download never starts, check:
1. Is `ensureInitialized()` being called? (should happen on first prompt)
2. Is `pendingAdapterConfig` set? (check in service worker)
3. Are there any errors when calling `adapter.initialize()`?
4. Is the dynamic import failing in window context too?

If configuration doesn't persist:
1. Check chrome.storage.sync in DevTools
2. Look for "saveConfig" logs
3. Look for "loadConfig" logs on extension reload
4. Check for storage quota errors
