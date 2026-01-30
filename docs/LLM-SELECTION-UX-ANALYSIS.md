# LLM Selection UX Analysis

## Context

Oryn is a browser automation agent that uses LLMs to understand web pages and execute tasks. Users need to select and configure an LLM provider before the agent can work.

Current implementation: 6 adapter types (Chrome AI, WebLLM, wllama, OpenAI, Claude, Gemini) with multiple models each.

---

## User Personas

### 1. **Casual User** (60% of users)
- **Goals**: Get automation working quickly
- **Technical level**: Basic
- **Concerns**: "Will it cost money?", "Is this safe?"
- **Hardware**: Standard laptop, recent Chrome
- **Behavior**: Wants defaults, avoids configuration
- **Pain point**: Overwhelmed by options

### 2. **Privacy Advocate** (20% of users)
- **Goals**: Keep everything local, no data sharing
- **Technical level**: Intermediate to Advanced
- **Concerns**: "Does this send my data to the cloud?"
- **Hardware**: Varies (may have older systems)
- **Behavior**: Reads documentation, willing to wait for downloads
- **Pain point**: Unclear which options are truly local

### 3. **Power User/Developer** (15% of users)
- **Goals**: Best performance, test different configurations
- **Technical level**: Advanced
- **Concerns**: "What's the fastest/most accurate option?"
- **Hardware**: High-end, often has API keys
- **Behavior**: Switches frequently, benchmarks options
- **Pain point**: Switching is cumbersome, no quick comparison

### 4. **Enterprise/Restricted User** (5% of users)
- **Goals**: Work in air-gapped or restricted network
- **Technical level**: Varies
- **Concerns**: "Will this work offline?", "Is this approved?"
- **Hardware**: Locked-down corporate machines
- **Behavior**: Needs local-only, pre-approved solutions
- **Pain point**: Cannot download large models, limited hardware

---

## Use Cases

### Primary Use Cases

**UC1: Run agent for first time**
- User installs extension
- Opens side panel
- Tries to start agent
- Gets error: "No LLM configured"
- Must configure before using

**UC2: Quick web automation task**
- User wants to fill a form / extract data
- Needs fast response (< 2s)
- Doesn't want to pay API costs
- Task is simple (doesn't need GPT-4 quality)

**UC3: Complex multi-step automation**
- User has complex workflow (e-commerce checkout, multi-page forms)
- Needs high accuracy to avoid errors
- Willing to wait longer / pay for quality
- Task is important (worth using paid API)

**UC4: Offline/private automation**
- User on airplane, restricted network, or sensitive data
- Cannot use remote APIs
- Must work entirely locally
- Privacy is critical

**UC5: Development and testing**
- Developer testing automation scripts
- Needs to compare model behaviors
- Wants to switch quickly between options
- Iterating on prompts/trajectories

---

## Scenarios

### Scenario A: First-Time Setup (Casual User)

**Context**: Just installed extension, no configuration

**Current Flow**:
1. User opens side panel
2. Tries to start agent → Error: "No LLM configured"
3. Clicks "Configure LLM" → Opens options page
4. Sees 6 adapter cards with technical jargon
5. **Decision paralysis**: Which one? What's the difference?
6. Randomly picks OpenAI (sees "Most capable")
7. Needs API key → Doesn't have one
8. Gives up or spends 20 minutes researching

**Problems**:
- No guidance on what to choose
- No working default
- API key requirement is a blocker
- Technical terminology confusing
- Too many choices upfront

**Ideal Flow**:
1. User opens side panel
2. Extension auto-detects Chrome AI is available
3. **Agent works immediately with Chrome AI** (no config needed)
4. User sees: "Using Chrome AI (free, local) - [Change]"
5. Can optionally explore other options later

---

### Scenario B: Privacy-Conscious User Wants Best Local Option

**Context**: Has read about privacy, wants best local LLM

**Current Flow**:
1. Opens LLM configuration
2. Sees Chrome AI, WebLLM, wllama, OpenAI, Claude, Gemini
3. **Not clear which are local vs remote**
4. Reads descriptions carefully
5. Sees WebLLM says "local" and "high quality"
6. Selects WebLLM → Picks Llama-3-8B (4.5GB)
7. Clicks Save → Suddenly downloading 4.5GB
8. **Surprise**: Didn't realize download size
9. Waits 15 minutes on slow connection
10. May abandon if connection drops

**Problems**:
- Local vs remote not clearly categorized
- Download size not prominent enough
- No size-based recommendations
- Can't estimate download time
- No way to cancel/resume download

**Ideal Flow**:
1. Opens LLM configuration
2. Sees clear sections: "LOCAL (Private)" vs "REMOTE (Requires API)"
3. In local section: Chrome AI (instant), WebLLM (download 1.5-4.5GB), wllama (download 0.7-1.6GB)
4. **Each model shows**: Size, quality, speed, download time estimate
5. Recommendations: "For your connection speed (10 Mbps), we recommend Gemma-2B (1.5GB, ~2 min download)"
6. User selects, sees confirmation: "This will download 1.5GB. Continue?"
7. Progress shown with ability to pause/resume
8. Can use Chrome AI while downloading in background

---

### Scenario C: Power User Wants Quick Switching

**Context**: Testing automation scripts, wants to compare WebLLM vs Claude

**Current Flow**:
1. Currently using WebLLM
2. Opens configuration page
3. Selects Claude adapter
4. Enters API key
5. Saves configuration
6. **WebLLM model unloads** (loses 4.5GB in memory)
7. Tests with Claude
8. Wants to switch back to WebLLM
9. Must reconfigure → **Model redownloads** (another 10 min wait)
10. Very frustrating iteration cycle

**Problems**:
- Switching requires full reconfiguration
- Models don't stay cached
- No quick toggle between configured adapters
- No side-by-side comparison
- Lost time redownloading

**Ideal Flow**:
1. Configures multiple adapters once (WebLLM, Claude)
2. **Quick switcher in side panel**: Dropdown showing all configured adapters
3. Selects from dropdown → Switches instantly (keeps both in memory if space allows)
4. Side panel shows: "Using Claude - [↔️ Switch]"
5. Can A/B test same task with different models
6. Models stay loaded until explicitly cleared

---

### Scenario D: Limited Hardware User

**Context**: Old laptop, 4GB RAM, no WebGPU, slow internet

**Current Flow**:
1. Opens configuration
2. Sees WebLLM (requires WebGPU) → Not available
3. Tries wllama → Models are 700MB-1.6GB
4. Downloads TinyLlama (700MB) on slow connection → Takes 15 minutes
5. **Model loads but system becomes slow** (high RAM usage)
6. Agent barely works, system laggy
7. Frustrated, may uninstall

**Problems**:
- No hardware capability detection upfront
- No warnings about RAM requirements
- No "lite" mode for constrained systems
- Models may not fit in available memory
- No graceful degradation

**Ideal Flow**:
1. Extension detects: No WebGPU, 4GB RAM, slow connection
2. **Automatic recommendation**: "Based on your system, we recommend Chrome AI (built-in, no download)"
3. Alternatively: "Use OpenAI API (no local resources needed)"
4. Shows warning if selecting large model: "⚠️ This model requires 6GB RAM. You have 4GB available."
5. Suggests alternatives automatically
6. Can use lightweight remote API as fallback

---

## Current UX Problems

### 1. **No Smart Defaults**
- User must explicitly configure before first use
- No auto-detection of best available option
- Cold start problem

### 2. **Poor Information Architecture**
- 6 adapters shown flat, no categorization
- Local vs remote not clearly separated
- No priority/recommendation shown
- Technical descriptions assume knowledge

### 3. **Hidden Costs (Time & Data)**
- Download sizes not prominent
- No download time estimates
- No warning before large downloads
- Bandwidth costs not considered

### 4. **No Onboarding**
- No wizard for first-time setup
- No explanation of trade-offs
- No "recommended for you" flow
- Users must understand LLMs to choose

### 5. **Inflexible Switching**
- Must reconfigure to change adapters
- Models don't stay cached
- No quick toggle
- Slow iteration for testing

### 6. **Missing Context**
- No hardware capability warnings
- No "why this matters" explanations
- No performance comparisons
- No cost estimates (API vs free)

### 7. **Poor Progress Feedback**
- Download progress only in console (before our fix)
- No background download indicator
- No estimated time remaining
- Can't check download status from side panel

### 8. **Configuration Persistence Unclear**
- User doesn't know if config survives extension reload
- No indication of "current" vs "configured but not active"
- No config version/history

---

## Proposed UX Improvements

### Improvement 1: Smart Auto-Configuration

**Default Behavior**:
```
On first install:
1. Detect available adapters (Chrome AI, WebGPU, etc.)
2. Auto-select best free option:
   - Chrome AI if available → Use immediately
   - Else wllama (small model) → Offer quick setup
   - Else → Prompt for API key OR download
3. Agent works immediately (no config required)
4. Show in side panel: "Using Chrome AI ✓ - Change"
```

**Benefits**:
- Zero friction for casual users
- Immediate functionality
- Can explore options later

---

### Improvement 2: Categorized Selection

**New UI Structure**:
```
┌─────────────────────────────────────────────────┐
│ 🤖 Choose Your LLM                              │
├─────────────────────────────────────────────────┤
│ 🚀 INSTANT & FREE (Recommended)                 │
│ ┌─────────────────────────────────────────────┐ │
│ │ ⚡ Chrome AI                        [Select]│ │
│ │ Built-in browser AI. Fast, private, free.   │ │
│ │ Quality: Good | Speed: <1s | Cost: Free     │ │
│ └─────────────────────────────────────────────┘ │
│                                                  │
│ 💾 DOWNLOAD ONCE, USE FOREVER                   │
│ ┌─────────────────────────────────────────────┐ │
│ │ 🚀 WebLLM (GPU Required)          [Select] │ │
│ │ High-quality local AI. Download 1.5-4.5GB   │ │
│ │ Quality: Excellent | Speed: <2s | Cost: Free│ │
│ │ 📥 Choose size: [Gemma 1.5GB▾] (~2min)     │ │
│ └─────────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────────┐ │
│ │ 🦙 wllama (CPU, Works Anywhere)   [Select] │ │
│ │ Compatible fallback. Download 0.7-1.6GB     │ │
│ │ Quality: Good | Speed: 3-8s | Cost: Free    │ │
│ │ 📥 Choose size: [TinyLlama 669MB▾] (~1min) │ │
│ └─────────────────────────────────────────────┘ │
│                                                  │
│ ☁️ CLOUD APIs (Best Quality)                   │
│ ┌─────────────────────────────────────────────┐ │
│ │ 🤖 OpenAI GPT-4                   [Select] │ │
│ │ Most capable. Requires API key.             │ │
│ │ Quality: Best | Speed: 1-2s | Cost: ~$0.03 │ │
│ │ 🔑 [Enter API Key]                          │ │
│ └─────────────────────────────────────────────┘ │
│                                                  │
│ [Save Configuration] [Test Current]             │
└─────────────────────────────────────────────────┘
```

**Benefits**:
- Clear categorization (instant/download/cloud)
- Upfront cost/time/quality comparison
- Download size and time visible
- Progressive disclosure (collapse non-selected)

---

### Improvement 3: First-Run Wizard

**Wizard Flow**:
```
Step 1: What matters most?
  • Speed - Get started instantly (Chrome AI)
  • Privacy - Everything stays local (WebLLM/wllama)
  • Quality - Best results, willing to pay (GPT-4)
  • Balanced - Smart auto-selection

Step 2: Are you willing to download models?
  • Yes - Download now for best quality (WebLLM 1.5GB)
  • Maybe later - Start with Chrome AI, download in background
  • No - Use Chrome AI or cloud APIs only

Step 3: Hardware check
  ✓ Chrome with WebGPU detected
  ✓ 8GB RAM available
  ⚠️ Internet connection: Moderate (5 Mbps)

  Recommendation: Start with Chrome AI, download WebLLM Gemma-2B in background
  Estimated download: 3 minutes

[Start Using Oryn] [Customize Further]
```

**Benefits**:
- Guided setup for new users
- Learns user preferences
- Hardware-aware recommendations
- Option to skip for advanced users

---

### Improvement 4: Quick Switcher

**Side Panel Integration**:
```
┌─────────────────────────────────────┐
│ 🤖 Ralph Agent                      │
├─────────────────────────────────────┤
│ LLM: [Chrome AI ▾] ⚙️              │
│      ├─ Chrome AI (current)         │
│      ├─ WebLLM (ready)              │
│      ├─ wllama (ready)              │
│      ├─ OpenAI (needs key)          │
│      └─ Configure more...           │
│                                      │
│ Task: [Fill out form___________]    │
│ [▶ Start Agent]                     │
└─────────────────────────────────────┘
```

**Benefits**:
- Switch without leaving side panel
- See which adapters are ready
- Maintain multiple configured adapters
- Quick iteration for testing

---

### Improvement 5: Download Management

**Background Download UI**:
```
Side Panel Badge:
┌─────────────────────────────────────┐
│ 📥 Downloading WebLLM... 45%         │
│ [View] [Pause]                       │
└─────────────────────────────────────┘

Configuration Page:
┌─────────────────────────────────────┐
│ 📥 Active Downloads (1)              │
│ ┌─────────────────────────────────┐ │
│ │ WebLLM Gemma-2B                 │ │
│ │ ▓▓▓▓▓▓▓▓░░░░░░ 45% (680MB/1.5GB)│ │
│ │ ~1 minute remaining              │ │
│ │ [Pause] [Cancel]                 │ │
│ └─────────────────────────────────┘ │
│                                      │
│ 💾 Downloaded Models (2)             │
│ • Chrome AI (built-in)               │
│ • wllama TinyLlama (669MB) [Delete] │
└─────────────────────────────────────┘
```

**Benefits**:
- See download status anywhere
- Pause/resume capability
- Manage downloaded models
- Clear disk usage information

---

### Improvement 6: Contextual Recommendations

**Smart Suggestions**:
```
When starting agent for simple task:
┌─────────────────────────────────────┐
│ Task: "Click the login button"      │
│                                      │
│ 💡 Tip: This is a simple task       │
│ Chrome AI (fast, free) is perfect!  │
│ Currently using: GPT-4 ($$$)        │
│                                      │
│ [Switch to Chrome AI] [Keep GPT-4]  │
└─────────────────────────────────────┘

When hardware changes:
┌─────────────────────────────────────┐
│ ⚠️ WebGPU no longer available       │
│ (Chrome update or settings change)  │
│                                      │
│ WebLLM won't work. Switch to:       │
│ • wllama (CPU, similar quality)     │
│ • Chrome AI (faster, built-in)      │
│                                      │
│ [Auto-Switch to wllama]             │
└─────────────────────────────────────┘
```

**Benefits**:
- Proactive guidance
- Cost/quality optimization
- Adapts to context
- Teaches users about options

---

### Improvement 7: Comparison Mode

**A/B Testing UI**:
```
┌─────────────────────────────────────────────────┐
│ 🔬 Compare Models                               │
├─────────────────────────────────────────────────┤
│ Run the same task with different models:        │
│                                                  │
│ Select models to compare:                       │
│ ☑ Chrome AI                                     │
│ ☑ WebLLM Phi-3                                  │
│ ☐ GPT-4                                         │
│                                                  │
│ Task: "Find the price of this product"          │
│                                                  │
│ [Run Comparison]                                │
│                                                  │
│ Results:                                         │
│ ┌─────────────────┬──────┬────────┬──────────┐ │
│ │ Model           │Time  │ Result │ Accuracy │ │
│ ├─────────────────┼──────┼────────┼──────────┤ │
│ │ Chrome AI       │0.8s  │ $24.99 │ ✓        │ │
│ │ WebLLM Phi-3    │1.2s  │ $24.99 │ ✓        │ │
│ │ GPT-4           │-     │ -      │ -        │ │
│ └─────────────────┴──────┴────────┴──────────┘ │
└─────────────────────────────────────────────────┘
```

**Benefits**:
- Data-driven decisions
- See actual performance differences
- Justify paid API usage
- Optimize for specific tasks

---

## Complete User Journeys

### Journey 1: Casual User - Zero Config

**Goal**: Fill out a web form quickly

```
1. Install extension
   ↓
2. Open side panel
   ↓
3. [Extension auto-detects Chrome AI]
   ↓
4. Side panel shows: "Using Chrome AI ✓"
   ↓
5. Enter task: "Fill out the contact form"
   ↓
6. Click "Start Agent"
   ↓
7. Agent works immediately
   ✓ Success - No configuration needed
```

**Touchpoints**: 1 (side panel)
**Time to first use**: < 30 seconds
**Friction points**: None

---

### Journey 2: Privacy User - Local Setup

**Goal**: Use best local model, no cloud

```
1. Install extension
   ↓
2. Open side panel → Click "Configure LLM"
   ↓
3. Wizard appears: "What matters most?"
   Select: "Privacy - Everything stays local"
   ↓
4. Wizard: "Are you willing to download?"
   Select: "Yes, download now"
   ↓
5. Wizard shows: "Recommended: WebLLM Gemma-2B (1.5GB)"
   Hardware check: ✓ WebGPU available, ✓ 8GB RAM
   Download time: ~3 minutes on your connection
   ↓
6. Click "Download & Configure"
   ↓
7. Progress modal appears with download bar
   Can close and use Chrome AI while downloading
   ↓
8. [3 minutes later] Notification: "WebLLM ready!"
   ↓
9. Side panel auto-switches to WebLLM
   ↓
10. Start using agent with local WebLLM
    ✓ Success - Local, private, high quality
```

**Touchpoints**: 2 (wizard + progress modal)
**Time to first use**: < 1 min (Chrome AI), 3-5 min (WebLLM)
**Friction points**: Download time (but managed well)

---

### Journey 3: Power User - Multi-Model Testing

**Goal**: Compare Chrome AI vs GPT-4 for accuracy

```
1. Already using Chrome AI
   ↓
2. Open configuration page
   ↓
3. In "Cloud APIs" section, add OpenAI
   Enter API key → Save
   ↓
4. Return to side panel
   ↓
5. Click LLM dropdown: [Chrome AI ▾]
   See: Chrome AI (current), OpenAI GPT-4 (ready)
   ↓
6. Select task: "Extract all email addresses from this page"
   ↓
7. Run with Chrome AI → Result: 3 emails
   ↓
8. Click dropdown → Switch to OpenAI GPT-4
   ↓
9. Run same task → Result: 5 emails
   ↓
10. Compare: GPT-4 found 2 more (in footer)
    Decision: Use GPT-4 for extraction tasks
    ✓ Success - Easy comparison, data-driven decision
```

**Touchpoints**: 2 (config page + side panel)
**Time to compare**: < 2 minutes
**Friction points**: None (both models ready)

---

### Journey 4: Limited Hardware User - Graceful Degradation

**Goal**: Use automation on old laptop (4GB RAM, no WebGPU)

```
1. Install extension
   ↓
2. Open configuration
   ↓
3. [Extension detects: No WebGPU, 4GB RAM]
   ↓
4. UI automatically hides WebLLM option
   Shows warning: "WebGPU not available"
   ↓
5. Recommended options highlighted:
   - Chrome AI (built-in, 0MB)
   - OpenAI API (no local resources)
   ↓
6. User selects Chrome AI
   ↓
7. Clicks Save → Works immediately
   ↓
8. [Later] User tries wllama out of curiosity
   ↓
9. Warning appears: "⚠️ TinyLlama requires 2GB RAM. You have 1.5GB available. Performance may be slow."
   ↓
10. User cancels, sticks with Chrome AI
    ✓ Success - Prevented poor experience
```

**Touchpoints**: 1 (config page with warnings)
**Time to first use**: < 1 minute
**Friction points**: Hardware limitations (but handled gracefully)

---

## Key UX Principles

### 1. **Progressive Disclosure**
- Don't show all options upfront
- Start simple, reveal complexity as needed
- Casual users see simple interface
- Power users can drill down

### 2. **Intelligent Defaults**
- Auto-detect best option
- Work out-of-the-box when possible
- Configuration is optional, not required

### 3. **Clear Mental Models**
- Categorize: Instant / Download / Cloud
- Visualize: Size, time, cost trade-offs
- Explain: Why choose one over another

### 4. **Graceful Degradation**
- Detect hardware limitations early
- Prevent bad experiences proactively
- Offer appropriate alternatives

### 5. **Feedback & Control**
- Show progress for long operations
- Allow pause/resume/cancel
- Indicate current state clearly

### 6. **Contextual Guidance**
- Recommend based on task complexity
- Warn about costs (time, money, resources)
- Teach through use

---

## Implementation Priority

### P0 (Critical - Ship Blockers)
1. ✅ Smart auto-detection (Chrome AI default)
2. ✅ Download progress UI (done!)
3. ✅ Categorized selection (Local vs Cloud)
4. Hardware capability detection

### P1 (High - Next Release)
1. Quick switcher in side panel
2. First-run wizard
3. Background download indicator
4. Download pause/resume

### P2 (Medium - Nice to Have)
1. Comparison mode
2. Contextual recommendations
3. Model management (delete, re-download)
4. Usage analytics (which model used most)

### P3 (Low - Future)
1. A/B testing framework
2. Performance benchmarking
3. Cost tracking (API usage)
4. Custom model uploads

---

## Success Metrics

### User Experience Metrics
- **Time to First Use**: < 30 seconds for 80% of users
- **Configuration Completion Rate**: > 90%
- **Wizard Completion Rate**: > 85%
- **Model Switch Frequency**: > 1 per session for power users

### Technical Metrics
- **Auto-Configuration Success**: > 95%
- **Download Completion Rate**: > 80%
- **Average Download Time**: < 5 minutes for Gemma-2B

### User Satisfaction
- **Post-Setup Survey**: "How easy was setup?" > 4/5
- **Support Tickets**: < 10% related to LLM configuration
- **Feature Adoption**: > 60% use local models

---

## Open Questions

1. **Should we bundle a tiny model with the extension?**
   - Pro: Instant local option, no download
   - Con: Increases extension size (20-50MB)
   - Compromise: Optional download during installation?

2. **How much to auto-switch vs let user control?**
   - Auto-switch on hardware changes?
   - Auto-recommend better options?
   - Risk: User loses control, unexpected behavior

3. **Should we persist multiple configured adapters?**
   - Pro: Quick switching, good for testing
   - Con: Memory usage, complexity
   - Compromise: Keep 2-3 most recent?

4. **How to handle model updates?**
   - Auto-update downloaded models?
   - Notify user of newer versions?
   - Allow version pinning?

5. **Should free tier have limits?**
   - Rate limit local model usage?
   - Encourage API key for heavy use?
   - How to communicate fairly?

---

## Conclusion

The current implementation is **technically complete** but has **significant UX gaps**:

- ✅ **Good**: Multiple adapter options, download progress
- ⚠️ **Needs Work**: Onboarding, categorization, smart defaults
- ❌ **Missing**: Hardware detection, contextual guidance, quick switching

**Recommended Next Steps**:
1. Implement smart auto-configuration (Chrome AI default)
2. Add hardware capability detection and warnings
3. Reorganize UI with clear categorization
4. Add quick switcher to side panel
5. Build first-run wizard for guided setup

This will transform the experience from "technical configuration tool" to "works out of the box, customizable when needed."
