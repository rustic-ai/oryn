# Oryn-W Extension - Troubleshooting Guide

## Common Issues and Solutions

### "Could not establish connection. Receiving end does not exist"

**Symptoms**: Error appears when trying to execute commands or use agent mode.

**Cause**: The content script is not loaded on the current page, or you're on a restricted page.

**Solutions**:

1. **Check the Page Type**:
   - ❌ **Won't work** on: `chrome://`, `chrome-extension://`, `edge://`, `about:`, Chrome Web Store
   - ✅ **Will work** on: Regular websites (`http://` or `https://`)

2. **Navigate to a Regular Website**:
   ```
   Try these sites for testing:
   - https://www.google.com
   - https://www.example.com
   - https://news.ycombinator.com
   ```

3. **Reload the Extension**:
   - Go to `chrome://extensions`
   - Find "Oryn Agent (WASM)"
   - Click the refresh icon
   - Reload the webpage you're testing on

4. **Reload the Webpage**:
   - Press `Ctrl+R` (or `Cmd+R` on Mac)
   - Or click the refresh button in your browser

5. **Check Content Script Injection**:
   - Open DevTools (F12)
   - Go to Console tab
   - Look for: `[CONTENT] Oryn Content Script Initialized`
   - If missing, the content script failed to load

**Auto-Fix**: The extension now automatically tries to inject the content script if it's missing. If you still see this error, the page doesn't allow content scripts.

---

### LLM Badge Shows "Not Configured"

**Symptoms**: LLM status shows gray "LLM: Not configured" badge.

**Cause**: No LLM adapter has been configured yet.

**Solution**:

1. **Configure an LLM**:
   - Click "Configure LLM" button in sidepanel
   - Choose an adapter:
     - **Chrome AI** (free, local) - Recommended for testing
     - **OpenAI** (requires API key)
     - **Claude** (requires API key)
     - **Gemini** (requires API key)
   - Click "Save Configuration"

2. **For Chrome AI**:
   - Requires Chrome 127+
   - Enable AI features in `chrome://flags`
   - Search for "Optimization Guide On Device Model"
   - Set to "Enabled"
   - Restart Chrome

3. **For Remote APIs**:
   - Get API key:
     - OpenAI: https://platform.openai.com/api-keys
     - Claude: https://console.anthropic.com/
     - Gemini: https://aistudio.google.com/
   - Enter key in configuration
   - Test connection

---

### "Agent Mode" Button is Disabled

**Symptoms**: Cannot click "Start Agent" button, it's grayed out.

**Cause**: No LLM is configured.

**Solution**: Configure an LLM first (see above).

---

### WASM Badge Shows "Error"

**Symptoms**: Red "WASM: Error" badge in sidepanel.

**Cause**: WASM module failed to load or initialize.

**Solutions**:

1. **Check Browser Console**:
   - Open DevTools (F12)
   - Look for WASM-related errors
   - Common issues:
     - WASM file not found
     - Loading blocked by CSP
     - Unsupported browser

2. **Verify WASM File Exists**:
   ```
   Extension should have: wasm/oryn_core_bg.wasm (2.0 MB)
   ```

3. **Reload Extension**:
   - Go to `chrome://extensions`
   - Click refresh icon on extension
   - Reload the page

4. **Check Browser Compatibility**:
   - Chrome/Chromium 88+
   - Edge (Chromium-based)
   - Brave, Opera, Vivaldi

5. **Clear Extension Cache**:
   ```
   1. chrome://extensions
   2. Remove extension
   3. Reload extension (Load unpacked)
   ```

---

### Agent Execution Fails Immediately

**Symptoms**: Agent shows error right after clicking "Start Agent".

**Cause**: Page validation failed or content script not available.

**Solutions**:

1. **Check Page Type** (see first issue above)

2. **Verify LLM is Working**:
   - LLM badge should be green
   - Try clicking "Configure LLM" → "Test Prompt"
   - Should get response

3. **Check Browser Console**:
   - Look for specific error messages
   - Agent errors are prefixed with `[Ralph Agent]`

4. **Try Simple Task First**:
   - Navigate to https://www.google.com
   - Enter task: "Search for cats"
   - Should type and click search

---

### Agent Gets Stuck or Loops

**Symptoms**: Agent repeats same action or doesn't complete task.

**Cause**: LLM is producing invalid commands or task is unclear.

**Solutions**:

1. **Clear History**:
   - Click "Clear History" button
   - Start fresh task

2. **Rephrase Task**:
   - Be more specific
   - Break into smaller steps
   - Examples:
     - ❌ "Buy something"
     - ✅ "Search for blue backpacks and click the first result"

3. **Check Max Iterations**:
   - Default: 10 iterations
   - Agent stops automatically
   - Reduce complexity if hitting limit

4. **Review LLM Quality**:
   - Some LLMs work better than others
   - Try different adapter
   - OpenAI GPT-4 is most reliable

---

### Trajectory Store Issues

**Symptoms**: Trajectories not saving or retrieving.

**Cause**: IndexedDB errors or storage quota exceeded.

**Solutions**:

1. **Check Storage Quota**:
   ```javascript
   // In browser console
   navigator.storage.estimate().then(console.log)
   ```

2. **Clear Trajectories**:
   - Use trajectory manager (if implemented)
   - Or clear IndexedDB:
     ```javascript
     // In console
     indexedDB.deleteDatabase('OrynTrajectories')
     ```

3. **Reload Seed Trajectories**:
   - Clear database
   - Reload extension
   - Seed trajectories auto-load (20 examples)

4. **Export Backup**:
   ```javascript
   // Save trajectories before clearing
   chrome.runtime.sendMessage({type: 'trajectory_export'}, (response) => {
     console.log(response.data); // Copy this
   });
   ```

---

### Commands Execute on Wrong Page

**Symptoms**: Commands affect different tab than expected.

**Cause**: Multiple tabs open, wrong tab active.

**Solutions**:

1. **Ensure Correct Tab Active**:
   - Click on the tab you want to automate
   - Then open sidepanel

2. **Close Other Tabs**:
   - Temporarily close other tabs
   - Execute commands
   - Reopen tabs after

3. **Check Tab ID**:
   - Extension uses currently active tab
   - Switching tabs during execution may cause issues

---

### Performance Issues / Slow Execution

**Symptoms**: Commands take a long time to execute.

**Cause**: Large page DOM, slow LLM, or network latency.

**Solutions**:

1. **Check LLM Response Time**:
   - Chrome AI: <1s
   - Remote APIs: 0.5-2s
   - If slower, check network

2. **Reduce Page Complexity**:
   - Test on simpler pages first
   - Large SPAs may scan slower

3. **Check Network**:
   - Remote APIs require internet
   - Use Chrome AI for offline

4. **Monitor Console**:
   - Check for slow operations
   - Look for timeouts

---

### API Key Errors

**Symptoms**: "API key validation failed" or "Unauthorized" errors.

**Cause**: Invalid, expired, or incorrectly entered API key.

**Solutions**:

1. **Verify API Key**:
   - Check for typos
   - Ensure no extra spaces
   - Key should start with:
     - OpenAI: `sk-`
     - Claude: `sk-ant-`
     - Gemini: Alphanumeric

2. **Check Key Validity**:
   - Go to provider's dashboard
   - Verify key is active
   - Check usage limits/quota

3. **Regenerate Key**:
   - Create new API key
   - Delete old key
   - Update in extension

4. **Test Connection**:
   - Click "Configure LLM"
   - Enter key
   - Click "Test Prompt"
   - Should get response

---

### Extension Updates Not Working

**Symptoms**: Extension doesn't update when new version available.

**Cause**: Manual extension loading or update mechanism disabled.

**Solutions**:

1. **For Unpacked Extension**:
   - Rebuild: `./scripts/build-extension-w.sh`
   - Go to `chrome://extensions`
   - Click refresh icon on extension

2. **For Chrome Web Store Version**:
   - Chrome auto-updates
   - Or manually: `chrome://extensions` → "Update"

3. **For CRX Version**:
   - Download new CRX
   - Remove old extension
   - Install new CRX

---

## Debugging Tips

### Enable Verbose Logging

Open browser console and run:
```javascript
// Enable detailed logs
localStorage.setItem('oryn_debug', 'true');

// Disable
localStorage.removeItem('oryn_debug');
```

### Check Extension State

```javascript
// Get LLM status
chrome.runtime.sendMessage({type: 'llm_status'}, console.log);

// Get agent status
chrome.runtime.sendMessage({type: 'agent_status'}, console.log);

// Get trajectory stats
chrome.runtime.sendMessage({type: 'trajectory_stats'}, console.log);

// Get extension status
chrome.runtime.sendMessage({type: 'get_status'}, console.log);
```

### Monitor Network Requests

1. Open DevTools (F12)
2. Go to Network tab
3. Filter by "fetch/XHR"
4. Watch API calls to LLM providers
5. Check status codes and response times

### Inspect IndexedDB

1. Open DevTools (F12)
2. Go to Application tab
3. Expand "IndexedDB"
4. Find "OrynTrajectories"
5. Inspect stored data

---

## Getting Help

### Before Reporting Issues

1. **Try these first**:
   - Reload the webpage
   - Reload the extension
   - Clear browser cache
   - Try a different website
   - Check browser console

2. **Gather information**:
   - Chrome version: `chrome://version`
   - Extension version: Check manifest.json
   - Error messages from console
   - Steps to reproduce

### Report Issues

**GitHub Issues**: https://github.com/anthropics/oryn/issues

**Include**:
- Browser and version
- Extension version
- Complete error messages
- Steps to reproduce
- Screenshots (if applicable)
- Console logs

**Template**:
```markdown
**Browser**: Chrome 127.0.6533.89

**Extension Version**: 0.1.0

**Error**:
[Paste error message here]

**Steps to Reproduce**:
1. Navigate to example.com
2. Click "Agent Mode"
3. Enter task: "Search for cats"
4. Click "Start Agent"

**Expected**: Agent should search
**Actual**: Error appears

**Console Logs**:
[Paste relevant logs]

**Screenshots**: [Attach if helpful]
```

---

## Known Limitations

1. **Page Restrictions**:
   - Cannot run on chrome:// pages
   - Cannot run on chrome-extension:// pages
   - Cannot run on Chrome Web Store
   - Cannot run on some enterprise-blocked sites

2. **Chrome AI Availability**:
   - Requires Chrome 127+
   - May need feature flags enabled
   - Model download required (first time)
   - Not available in all regions

3. **Task Complexity**:
   - Max 10 iterations by default
   - Complex multi-page flows may fail
   - Some dynamic sites harder to automate

4. **LLM Quality**:
   - Depends on selected provider
   - Results may vary
   - Some tasks need better prompting

5. **Storage Limits**:
   - IndexedDB quota limits
   - Typically 50-100 MB available
   - May need to clear old trajectories

---

## Frequently Asked Questions

**Q: What pages can I use this on?**
A: Any regular website (http:// or https://). Not on chrome://, chrome-extension://, or Chrome Web Store pages.

**Q: Do I need an API key?**
A: Only for remote LLMs (OpenAI, Claude, Gemini). Chrome AI is free and local.

**Q: Is my data sent to servers?**
A: Only if you use remote LLMs. With Chrome AI, everything stays local.

**Q: Why does Chrome AI take so long first time?**
A: It needs to download the Gemini Nano model (~1.5 GB). This is one-time.

**Q: Can I use this offline?**
A: Yes, with Chrome AI. Remote APIs require internet.

**Q: How do I update trajectories?**
A: They auto-save on successful tasks. Export/import available via console.

**Q: Can I use multiple LLMs?**
A: Yes, switch in configuration. Only one active at a time.

**Q: Is there a usage limit?**
A: Chrome AI: No limit. Remote APIs: Depends on your plan.

---

## Quick Fixes Checklist

When something doesn't work:

- [ ] Are you on a regular website (http:// or https://)?
- [ ] Is LLM badge green?
- [ ] Is WASM badge green?
- [ ] Did you reload the page?
- [ ] Did you reload the extension?
- [ ] Did you check browser console for errors?
- [ ] Is your API key valid (for remote LLMs)?
- [ ] Are you using a supported browser?
- [ ] Is JavaScript enabled on the page?

If all checked and still issues, see "Getting Help" section above.

---

**Last Updated**: January 29, 2026
**Extension Version**: 0.1.0
