# Oryn-W Extension Packing & Distribution Guide

## Overview

This guide covers how to pack and distribute the Oryn-W extension in various formats:
- **ZIP Package**: For Chrome Web Store submission
- **CRX Package**: For direct distribution and enterprise deployment
- **Unpacked**: For development and testing

---

## Quick Start

### Pack Extension for Distribution

```bash
# Build extension first (if not already built)
./scripts/build-extension-w.sh

# Pack extension
./scripts/pack-extension-w.sh
```

This creates:
- `dist/oryn-w-0.1.0.zip` - Ready for Chrome Web Store
- `dist/oryn-w-0.1.0.txt` - Package information
- `dist/oryn-w-0.1.0.sha256` - Checksum for verification

---

## Distribution Methods

### Method 1: Chrome Web Store (Recommended)

**Best for**: Public distribution, automatic updates, user trust

**Steps**:

1. **Pack the Extension**:
   ```bash
   ./scripts/pack-extension-w.sh
   ```

2. **Create Developer Account**:
   - Go to [Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole)
   - Pay one-time $5 registration fee
   - Complete developer account setup

3. **Submit Extension**:
   - Click "New Item"
   - Upload `dist/oryn-w-0.1.0.zip`
   - Fill out store listing:
     - **Name**: Oryn Agent (WASM)
     - **Description**: AI-powered web automation agent with natural language task execution
     - **Category**: Productivity
     - **Language**: English
   - Upload screenshots (1280x800 or 640x400)
   - Upload promotional images (440x280 small, 920x680 large, 1400x560 marquee)
   - Set privacy policy URL
   - Submit for review

4. **Review Process**:
   - Initial review: 1-3 days
   - May require clarifications
   - Once approved, publicly available

**Store Listing Template**:

```
Name: Oryn Agent (WASM)

Short Description:
AI-powered web automation agent with natural language task execution

Detailed Description:
Oryn Agent is a browser automation extension that understands natural language commands and autonomously completes multi-step web tasks.

Features:
• Natural Language Task Execution - Describe what you want, let AI do it
• Multi-LLM Support - Works with Chrome AI, OpenAI, Claude, and Gemini
• Few-Shot Learning - Learns from successful task examples
• Autonomous Multi-Step Execution - Completes complex workflows
• Dual Mode Interface - OIL commands or Agent mode
• Local & Cloud AI - Choose privacy (Chrome AI) or power (remote APIs)

Use Cases:
• Automate repetitive web tasks
• Extract data from websites
• Fill out forms automatically
• Navigate complex web workflows
• Test web applications

Privacy:
• No data collection by extension
• LLM usage depends on selected provider
• Trajectory data stored locally in IndexedDB
• No server-side processing for local mode

Permissions:
• activeTab - To interact with current page
• scripting - To inject automation scripts
• storage - To save configuration and trajectories
• tabs - To manage browser tabs
• sidePanel - To show control interface
• API access - To connect to LLM providers

Support:
• Documentation: https://github.com/anthropics/oryn
• Issues: https://github.com/anthropics/oryn/issues
```

### Method 2: Direct CRX Distribution

**Best for**: Enterprise deployment, beta testing, custom distribution

**Steps**:

1. **Pack Extension with Chrome**:

   **Option A: Using Chrome UI**:
   ```
   1. Open chrome://extensions
   2. Enable "Developer mode"
   3. Click "Pack extension"
   4. Browse to: /home/rohit/work/dragonscale/oryn/extension-w
   5. Leave "Private key file" empty (first time)
   6. Click "Pack Extension"
   ```

   **Option B: Using Command Line**:
   ```bash
   # Linux/Mac
   google-chrome --pack-extension=/home/rohit/work/dragonscale/oryn/extension-w

   # Or using chromium
   chromium --pack-extension=/home/rohit/work/dragonscale/oryn/extension-w
   ```

2. **Output Files**:
   - `extension-w.crx` - The packed extension
   - `extension-w.pem` - Private key (**KEEP SAFE!**)

3. **Store Private Key Securely**:
   ```bash
   # Move to secure location
   mkdir -p ~/.oryn-keys
   mv extension-w.pem ~/.oryn-keys/
   chmod 600 ~/.oryn-keys/extension-w.pem

   # For future updates, use this key:
   google-chrome --pack-extension=extension-w \
                 --pack-extension-key=~/.oryn-keys/extension-w.pem
   ```

4. **Distribute CRX File**:
   - Upload to your website
   - Share via email/file sharing
   - Include installation instructions (see below)

**Installation Instructions for Users**:

```markdown
# Installing Oryn-W Extension (.crx)

## Method 1: Drag and Drop
1. Download oryn-w-0.1.0.crx
2. Open Chrome and go to chrome://extensions
3. Enable "Developer mode" (toggle in top-right)
4. Drag the .crx file into the extensions page
5. Click "Add extension" when prompted

## Method 2: Load via Developer Mode
1. Download oryn-w-0.1.0.crx
2. Open Chrome and go to chrome://extensions
3. Enable "Developer mode"
4. Click "Load unpacked"
5. Extract the .crx file (it's just a ZIP file with different extension)
6. Select the extracted folder

Note: Chrome may show warnings for extensions not from the Web Store.
This is normal for self-distributed extensions.
```

### Method 3: Unpacked Development Version

**Best for**: Development, testing, quick iterations

**Steps**:

1. **Build Extension**:
   ```bash
   ./scripts/build-extension-w.sh
   ```

2. **Load in Chrome**:
   ```
   1. Open chrome://extensions
   2. Enable "Developer mode"
   3. Click "Load unpacked"
   4. Select: /home/rohit/work/dragonscale/oryn/extension-w
   ```

3. **Update After Changes**:
   ```
   # Rebuild
   ./scripts/build-extension-w.sh

   # Then in chrome://extensions, click the refresh icon on the extension
   ```

---

## Package Contents

### Included Files

```
oryn-w-0.1.0/
├── manifest.json              # Extension configuration
├── background.js              # Service worker
├── sidepanel.html             # Main UI
├── sidepanel.js               # UI logic
├── content.js                 # Content script
├── scanner.js                 # DOM scanner
├── popup.html                 # Extension popup
├── popup.js                   # Popup logic
├── suppress_alerts.js         # Alert suppression
├── icons/                     # Extension icons
│   └── icon-128.svg
├── wasm/                      # WASM module
│   ├── oryn_core_bg.wasm      # OIL executor
│   ├── oryn_core.js           # JS bindings
│   └── *.d.ts                 # Type definitions
├── llm/                       # LLM adapters
│   ├── llm_adapter.js
│   ├── llm_manager.js
│   ├── chrome_ai_adapter.js
│   ├── openai_adapter.js
│   ├── claude_adapter.js
│   └── gemini_adapter.js
├── agent/                     # Ralph agent
│   ├── ralph_agent.js
│   ├── prompts.js
│   ├── trajectory_store.js
│   └── seed_trajectories.js
└── ui/                        # UI components
    ├── llm_selector.html
    ├── llm_selector.js
    └── llm_status_widget.html
```

### Excluded Files (Not Packed)

Development and documentation files are excluded from distribution:
- `*.md` documentation files
- `node_modules/`
- `package.json`, `package-lock.json`
- `.eslintrc.json`
- `test/` directory
- Build status files

---

## Version Management

### Updating Version Number

1. **Edit manifest.json**:
   ```json
   {
     "version": "0.2.0"
   }
   ```

2. **Rebuild and Repack**:
   ```bash
   ./scripts/build-extension-w.sh
   ./scripts/pack-extension-w.sh
   ```

3. **For Chrome Web Store Updates**:
   - Upload new ZIP file
   - Update "What's new" section
   - Submit for review

4. **For CRX Updates**:
   - Pack with same private key
   - Distribute new .crx file
   - Users will see update notification

### Version Numbering Scheme

Follow [Semantic Versioning](https://semver.org/):
- **Major** (1.0.0): Breaking changes, major new features
- **Minor** (0.1.0): New features, backward compatible
- **Patch** (0.0.1): Bug fixes, minor improvements

Current: `0.1.0` (initial release with Ralph Agent)

---

## Testing Before Distribution

### Pre-Distribution Checklist

- [ ] **Build succeeds without errors**
  ```bash
  ./scripts/build-extension-w.sh
  ```

- [ ] **All features work**
  - [ ] OIL mode executes commands
  - [ ] Agent mode completes tasks
  - [ ] LLM configuration saves
  - [ ] Trajectory store works
  - [ ] UI displays correctly

- [ ] **Test with clean profile**
  ```bash
  # Start Chrome with clean profile
  google-chrome --user-data-dir=/tmp/test-profile

  # Load extension
  # Test all features
  ```

- [ ] **Test different LLM adapters**
  - [ ] Chrome AI (if available)
  - [ ] OpenAI with test key
  - [ ] Claude with test key
  - [ ] Gemini with test key

- [ ] **Check console for errors**
  - Open DevTools console
  - Execute several tasks
  - Verify no errors

- [ ] **Verify permissions**
  - Check manifest.json permissions are minimal
  - Test extension works with specified permissions

- [ ] **Test on different websites**
  - [ ] Google (search)
  - [ ] Amazon (e-commerce)
  - [ ] GitHub (forms)
  - [ ] News sites (navigation)

- [ ] **Check package size**
  ```bash
  # Should be < 10 MB
  du -sh dist/oryn-w-*.zip
  ```

### Automated Testing

```bash
# Run all tests
./scripts/run-tests.sh

# Run E2E tests
./scripts/run-e2e-tests.sh --quick
```

---

## Security & Privacy

### Security Best Practices

1. **Code Review**:
   - Review all code before distribution
   - Check for hardcoded secrets
   - Verify no malicious code

2. **Minimize Permissions**:
   - Only request necessary permissions
   - Document why each permission is needed

3. **Secure API Keys**:
   - Never hardcode API keys
   - Store in chrome.storage (encrypted)
   - Warn users to keep keys private

4. **Content Security Policy**:
   ```json
   "content_security_policy": {
     "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"
   }
   ```

### Privacy Policy

Required for Chrome Web Store. Template:

```markdown
# Privacy Policy for Oryn Agent Extension

Last Updated: [DATE]

## Data Collection
Oryn Agent does not collect, store, or transmit any user data to external servers operated by the extension developer.

## Local Storage
- Configuration settings stored in chrome.storage.sync
- Task execution trajectories stored locally in IndexedDB
- No data leaves your device unless you explicitly configure remote LLM APIs

## Third-Party Services
When using remote LLM providers (OpenAI, Claude, Gemini):
- Data is sent to the selected provider's API
- Subject to that provider's privacy policy
- You control which provider is used
- Local mode (Chrome AI) keeps all data on device

## Permissions
- activeTab: To interact with web pages you're viewing
- scripting: To inject automation scripts
- storage: To save your preferences and learned tasks
- tabs: To manage browser tabs during automation
- sidePanel: To display the control interface

## Contact
For privacy concerns: [YOUR EMAIL]
```

---

## Troubleshooting

### Common Issues

**"Package is invalid"**:
- Check manifest.json is valid JSON
- Verify all required files are present
- Ensure icons are correct size (16x16, 48x48, 128x128)

**"CRX file is corrupt"**:
- Repack with correct private key
- Verify .pem file hasn't been modified
- Check Chrome version compatibility

**"Extension failed to load"**:
- Check browser console for specific errors
- Verify WASM file is present and valid
- Test with unpacked version first

**"Updates not working"**:
- Ensure same private key used for updates
- Check version number is incremented
- Verify update_url in manifest (for self-hosted)

---

## Distribution Channels

### Official Channels

1. **Chrome Web Store**:
   - URL: `https://chrome.google.com/webstore/detail/[EXTENSION_ID]`
   - Automatic updates
   - User reviews and ratings

2. **GitHub Releases**:
   ```bash
   # Create release
   git tag -a v0.1.0 -m "Release v0.1.0"
   git push origin v0.1.0

   # Upload dist/oryn-w-0.1.0.zip to release
   ```

3. **Self-Hosted**:
   - Host .crx file on your server
   - Provide update manifest
   - Users install via drag-and-drop

### Update Manifest (Self-Hosted)

If self-hosting updates, create `updates.xml`:

```xml
<?xml version='1.0' encoding='UTF-8'?>
<gupdate xmlns='http://www.google.com/update2/response' protocol='2.0'>
  <app appid='YOUR_EXTENSION_ID'>
    <updatecheck codebase='https://your-domain.com/oryn-w-0.1.0.crx' version='0.1.0' />
  </app>
</gupdate>
```

Add to manifest.json:
```json
{
  "update_url": "https://your-domain.com/updates.xml"
}
```

---

## Enterprise Deployment

### Group Policy Deployment

For managed Chrome deployments:

1. **Pack Extension**:
   ```bash
   ./scripts/pack-extension-w.sh
   google-chrome --pack-extension=extension-w
   ```

2. **Host CRX File**:
   ```
   https://your-company.com/extensions/oryn-w.crx
   ```

3. **Create Policy JSON**:
   ```json
   {
     "ExtensionInstallForcelist": [
       "YOUR_EXTENSION_ID;https://your-company.com/extensions/oryn-w.crx"
     ]
   }
   ```

4. **Deploy via Group Policy** (Windows):
   - Computer Configuration → Policies → Administrative Templates
   - → Google → Google Chrome → Extensions
   - Configure "Extension management settings"

5. **Deploy via MDM** (Mac/Linux):
   - Use Jamf, SCCM, or similar
   - Push policy configuration

---

## Appendix

### File Size Limits

- **Chrome Web Store**: 128 MB max
- **Current Package**: ~2.5 MB (well within limit)
- **WASM Module**: 2.0 MB (largest component)

### Browser Compatibility

- **Chrome**: 127+ (for Chrome AI, 88+ for basic features)
- **Chromium**: Latest stable
- **Edge**: Latest (Chromium-based)
- **Brave**: Latest
- **Opera**: Latest (Chromium-based)

### Support Resources

- **Documentation**: `extension-w/BUILD_COMPLETE.md`
- **Implementation**: `docs/RALPH-AGENT-IMPLEMENTATION-COMPLETE.md`
- **Issues**: GitHub Issues
- **Community**: Discussions tab

---

## Summary

**Quick Commands**:
```bash
# Build extension
./scripts/build-extension-w.sh

# Pack for distribution
./scripts/pack-extension-w.sh

# Test locally
# Load extension-w/ in chrome://extensions

# Submit to Chrome Web Store
# Upload dist/oryn-w-0.1.0.zip

# Create CRX for direct distribution
google-chrome --pack-extension=extension-w
```

**Distribution Options**:
1. ✅ Chrome Web Store (recommended)
2. ✅ Direct CRX distribution
3. ✅ Enterprise Group Policy
4. ✅ GitHub Releases
5. ✅ Self-hosted with updates

Choose the method that best fits your distribution needs!
