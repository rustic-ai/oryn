# Ralph Agent Prompt Review - Extension-W

## Current Prompt Structure

### System Prompt (`SYSTEM_PROMPT`)

```
You are a web automation agent that helps users complete tasks on websites using OIL (Oryn Intent Language).

OIL is a simple language for web automation with these commands:

BASIC ACTIONS:
- type "text" into "element"
- click "element"
- press Enter
- scroll to "element"

ELEMENT IDENTIFICATION:
Elements can be identified by:
- Text content: click "Submit"
- Type/role: click button, click link
- ID numbers: click [123]

OBSERVATION:
- observe - Scan the current page

IMPORTANT RULES:
1. Be specific when identifying elements
2. Use observe first if needed
3. Wait for pages to load
4. Complete tasks step by step
5. If complete, respond with "Status: COMPLETE"

Format:
Thought: <reasoning>
Action: <OIL command>

Or when done:
Thought: <why complete>
Status: COMPLETE
```

### User Prompt (`buildPrompt()`)

Structure:
1. Few-shot examples (from trajectories)
2. Current task
3. Current page state (URL, title, elements)
4. Previous actions (history)
5. Request for next action

---

## Critical Issues

### 1. **Element Context Is Severely Limited**

**Problem:**
```javascript
// Only shows 30 elements out of 200!
const elementsToShow = observation.elements.slice(0, 30);

// Vague notification
if (observation.elements.length > 30) {
    prompt += `\n... and ${observation.elements.length - 30} more elements\n`;
}
```

**What Agent Sees (Google Store Example):**
```
Available elements:
[1] button "Skip navigation"
[2] div "Phones\nEarbuds\nWatches..."
[3] link "Google Store home"
... (27 more navigation/layout elements)

... and 170 more elements
```

**What Agent Needs But Can't See:**
```
[45] link "Google Pixel 9 Pro - $999"
[46] link "Google Pixel 8a - $499"
[47] link "Pixel Fold - $1,799"
...
(All the actual product links are in elements 31-200)
```

**Impact:**
- Agent can't see products it's supposed to list
- Clicks "Phones" repeatedly because it doesn't see products already loaded
- Tries to find "List Phones" button that doesn't exist
- Gives up after 10 iterations

### 2. **No Guidance on How to See More**

**Current message:**
```
... and 170 more elements
```

**Agent doesn't know:**
- Can use `observe full` to see all elements
- Can use `observe from 30` to see next batch
- Can scroll down to load more
- Can filter by type (links, buttons, etc.)

**Should be:**
```
Showing first 30 of 200 elements (interactive only)
Element types: link:45, button:12, input:3, div:140

To see more:
- observe full (show all 200 elements)
- observe links (show all links)
- observe from 30 (show next 30 elements)
```

### 3. **Pattern Detection Is Not Actionable**

**Current:**
```
Detected patterns: Login Form, Search Box, Pagination
```

**Problems:**
- Doesn't explain what to do with patterns
- Doesn't link patterns to element IDs
- Agent doesn't understand implications

**Should be:**
```
Detected Patterns:
- Search Box: [15] input/search, [16] button "Search"
  → Use: type "query" into [15], click [16]

- Pagination: [180] link "Next", [181] link "Previous"
  → Use: click [180] to see more results

- Product Listing: Found 15 product cards (elements [45-59])
  → Products are already visible, extract text from links [45-59]
```

### 4. **Missing Critical Context**

**Current prompt doesn't include:**
- Page scroll position (might need to scroll)
- Viewport size (elements might be off-screen)
- Loading state (page still loading?)
- Network activity (AJAX loading more items?)
- Previous scan comparison (what changed?)

**Example of missing context:**
```
Agent clicks "Phones" → Page loads products
Agent runs observe → Sees [1-30] (navigation only)
Agent doesn't realize products loaded in [31-200]
```

### 5. **Element Presentation Is Not Smart**

**Current: First 30 elements (dumb slice)**
```
[1] button "Skip navigation"
[2] div "Phones\nEarbuds..."  (empty container)
[3] svg ""  (icon)
[4] div ""  (spacer)
[5] link "Google Store home"
...
[30] div ""  (another spacer)
```

**Should be: 30 most relevant elements**
```
[11] link "Phones" (current: clicked)
[45] link "Google Pixel 9 Pro - $999"
[46] link "Google Pixel 8a - $499"
[47] link "Pixel Fold - $1,799"
[48] button "Add to Cart" (for Pixel 9)
[49] link "Compare Phones"
[50] button "View Details"
...
(30 elements that matter for the task)
```

### 6. **Few-Shot Examples Are Too Generic**

**Current trajectory format:**
```
EXAMPLE 1:
Task: Buy a blue backpack
URL: https://example.com
Commands:
  - click "Shop"
  - type "blue backpack" into "search"
  - click "Search"
  - click "Add to Cart"
```

**Problems:**
- Doesn't show the observation state at each step
- Doesn't explain why each command was chosen
- Doesn't show what elements were visible
- Agent can't learn the decision pattern

**Should be:**
```
EXAMPLE 1: Extract product list
Task: List all laptops on the site
URL: https://store.example.com

Step 1:
  Observation: Homepage with navigation [Products, Support, Contact]
  Thought: Need to navigate to products section
  Action: click "Products"

Step 2:
  Observation: Products page showing categories [Laptops, Phones, Tablets]
  Thought: Found laptops category, click to see product list
  Action: click "Laptops"

Step 3:
  Observation: Laptop listing with 12 products visible [MacBook Pro $1999, Dell XPS...]
  Thought: Products are now visible, extract the list
  Action: extract text from links containing "$"
  Result: ["MacBook Pro - $1999", "Dell XPS 13 - $1299", ...]
  Status: COMPLETE
```

### 7. **No Self-Correction Mechanism**

**Current behavior:**
```
Iteration 4: click "List Phones" → FAILED (element not found)
Iteration 5: observe → Still can't see products
Iteration 6: click "Phones" → Again (already on phones page!)
Iteration 7: click "Phones" → Again (stuck in loop)
```

**Agent doesn't realize:**
- It already clicked "Phones" (should check history)
- "List Phones" doesn't exist (should try alternatives)
- Products are already on page (should look beyond first 30)

**Should have:**
```
Previous actions show:
- Iteration 1: click "Phones" → Success
- Iteration 4: click "List Phones" → FAILED

Error analysis:
- "List Phones" element not found
- Already on phones page
- Should look for product elements, not navigation

Suggested next action:
- observe links (to find product links)
- scroll down (to see more products)
- extract text (if products already visible)
```

### 8. **Temperature Too High for Deterministic Tasks**

**Current:**
```javascript
temperature: this.temperature,  // Default: 0.7
```

**Problem:**
- 0.7 is good for creative tasks
- Web automation needs consistency
- Same page → same action

**Should be:**
```javascript
temperature: 0.1,  // More deterministic
// Or dynamically adjust:
// - Navigation: 0.1 (predictable)
// - Extraction: 0.3 (slightly creative)
// - Search queries: 0.7 (flexible)
```

---

## Example: Current vs. Improved Prompt

### Current Prompt (Google Store "list phones" task)

```
SYSTEM: You are a web automation agent...
[OIL commands explained]

USER:
CURRENT TASK: list all phones from the site

CURRENT PAGE STATE:
URL: https://store.google.com/phones
Title: Google Store - Phones

Available elements:
[1] button "Skip navigation"
[2] div "Phones\nEarbuds\nWatches & Trackers..."
[3] link "Google Store home. Google Store logo."
[4] div ""
[5] svg ""
...
[30] link "Support"

... and 170 more elements

Detected patterns: Search Box, Pagination

PREVIOUS ACTIONS:
Thought: I should click Phones
Action: click "Phones"
Result: Success

Based on the task, examples, and current page state, what should be the next action?
```

**Agent Response:**
```
Thought: I have clicked on "Phones" and now need to observe the page to see the phone listings.
Action: observe
```

**Problem:** Agent runs observe but sees same 30 elements (navigation), not products!

---

### Improved Prompt (with all fixes)

```
SYSTEM: You are a web automation agent...
[Same OIL commands + extraction commands]

USER:
CURRENT TASK: list all phones from the site

SCAN SUMMARY:
- Total elements: 200
- Showing: 30 (viewport + interactive)
- Element types: link:45, button:12, input:3, div:140
- Scroll position: 0px / 2400px (more content below)

To see more elements:
- observe full          # Show all 200 elements
- observe links         # Show all 45 links
- observe from 30       # Show elements 31-60
- scroll down           # Load more content

CURRENT PAGE STATE:
URL: https://store.google.com/phones
Title: Google Store - Phones

Priority Elements (smart filtered):
[11] link "Phones" {current page}
[12] link "Earbuds"
[45] link "Google Pixel 9 Pro - Starting at $999" *
[46] link "Google Pixel 8a - Starting at $499" *
[47] link "Pixel Fold - Starting at $1,799" *
[48] link "Pixel Watch 3 - Starting at $349"
[49] button "Add to favorites" (for Pixel 9 Pro)
[50] button "Compare phones"
...
[60] link "View all accessories"

* = Product links (contains price)

Hidden elements include: 140 divs (layout), 33 more links, navigation items

Detected Patterns:
✓ Product Listing: 15 product cards detected (elements [45-59])
  → Suggested: Extract product names from links [45-59]
  → Products include prices and "Buy" buttons

✓ Search Box: [15] input/search, [16] button "Search"
  → Can refine product search

✓ Pagination: [180] link "Next page", [181] link "Previous"
  → More products available on next page

PREVIOUS ACTIONS (most recent first):
1. Thought: I should navigate to phones section
   Action: click "Phones"
   Result: ✓ Success (navigated to /phones)
   Page changed: Added 15 product cards

Analysis of current state:
- ✓ Already on phones page
- ✓ Products are visible (15 product links showing)
- ⚠ Clicking "Phones" again won't help
- → Should extract product information NOW

Based on the task, current state, and detected patterns, decide the next action.
The task is to LIST phones, and products are already visible.

What should you do?
```

**Agent Response (improved):**
```
Thought: I'm on the phones page and I can see 15 product links with prices. The task is to list phones, and products [45-59] contain phone names and prices. I should extract this information.
Action: observe links
```

Or even better:
```
Thought: Products are visible. Elements [45-59] are product links with prices. I should extract text from these links to list the phones.
Action: extract text from [45] to [59]
Status: COMPLETE (with extracted list)
```

---

## Recommended Fixes (Priority Order)

### 🔴 CRITICAL (Fix Immediately)

1. **Smart Element Filtering** (prompts.js:81)
   - Change from `slice(0, 30)` to intelligent filtering
   - Prioritize: links with text > buttons > inputs > layout divs
   - Filter out: empty divs, spacers, icons without text

2. **Add Scan Summary** (prompts.js:72-76)
   - Show total vs. displayed count
   - Break down by element type
   - Explain how to see more

3. **Show Pattern Actions** (prompts.js:105-117)
   - Link patterns to specific element IDs
   - Suggest concrete commands for each pattern
   - Explain what patterns mean

### 🟡 HIGH PRIORITY

4. **Improve Few-Shot Examples** (prompts.js:52-66)
   - Show observation at each step
   - Include reasoning for decisions
   - Demonstrate similar tasks (list items, extract data)

5. **Add Self-Correction** (ralph_agent.js:123-189)
   - Detect repeated actions (clicked same thing 3x)
   - Analyze failed commands (suggest alternatives)
   - Check if already on target page

6. **Lower Temperature** (ralph_agent.js:143)
   - Change default from 0.7 to 0.2
   - Or make task-specific (navigation: 0.1, extraction: 0.3)

### 🟢 MEDIUM PRIORITY

7. **Add Extraction Commands to OIL**
   - `extract text from links`
   - `extract products`
   - `list elements matching "price"`

8. **Add Scroll Context** (prompts.js:72-76)
   - Show scroll position
   - Indicate if more content below
   - Suggest scrolling when needed

9. **Show Element Changes** (prompts.js:120-128)
   - Compare to previous scan
   - Highlight what appeared/disappeared
   - Help agent understand page transitions

### 🔵 NICE TO HAVE

10. **Token Budget Optimization**
    - Calculate actual token usage
    - Adjust element count based on LLM limits
    - Chrome AI: ~2K tokens, GPT-4: ~8K tokens

11. **Streaming Observations**
    - Break large scans into chunks
    - Agent requests "next 30 elements" as needed
    - More interactive dialog

12. **Visual Indicators**
    - Mark current page/section with {current}
    - Show recently clicked elements with ⟲
    - Highlight products/targets with *

---

## Implementation Plan

### Phase 1: Emergency Fixes (2-3 hours)
- Smart element filtering in `prompts.js`
- Scan summary in `buildPrompt()`
- Pattern action suggestions
- Test on Google Store task

### Phase 2: Core Improvements (1 day)
- Enhanced few-shot format
- Self-correction logic
- Temperature tuning
- Retry with different strategy

### Phase 3: Advanced Features (2-3 days)
- Extraction commands in OIL
- Scroll awareness
- Diff/change detection
- Token optimization

---

## Testing Checklist

After implementing fixes, test these scenarios:

- [ ] Task: "List all phones" on Google Store
  - Should: Navigate → Observe → Extract products
  - Should NOT: Click "Phones" multiple times

- [ ] Task: "Search for blue backpacks"
  - Should: Use search box, not navigate
  - Should: Recognize search pattern

- [ ] Task: "Add item to cart"
  - Should: Click product → Click "Add to Cart"
  - Should: Confirm cart updated

- [ ] Task: "Fill login form"
  - Should: Recognize login pattern
  - Should: Fill email/password → Submit

- [ ] Error recovery
  - If command fails → Try alternative
  - If stuck → Request help or give up gracefully
  - If can't find element → Use observe to get more context

---

## Token Budget Analysis (Chrome AI)

**Current Prompt Size (Google Store example):**
```
System prompt:      ~400 tokens
Few-shot (3 traj):  ~600 tokens
Current page:       ~800 tokens (30 elements)
History:            ~200 tokens
Instructions:       ~100 tokens
-----------------------------------
Total:             ~2,100 tokens
```

**Chrome AI Context:** ~2,048 tokens (prompt + response)
**Problem:** Already at limit! Can't show more elements.

**Optimized Prompt:**
```
System prompt:      ~300 tokens (condensed)
Few-shot (2 traj):  ~400 tokens (shortened)
Scan summary:       ~50 tokens
Priority elements:  ~600 tokens (smart filtered)
Patterns:           ~100 tokens
History:            ~150 tokens (last 3 actions)
-----------------------------------
Total:             ~1,600 tokens ✓
```

**Budget remaining:** ~400 tokens for response = acceptable

---

## Conclusion

The current Ralph Agent prompt has **critical blindness** - it can't see what it needs to complete tasks. The agent is trying its best, but with only 30 elements (mostly navigation), it's like asking someone to find products while showing them only the store entrance.

**Key Takeaway:** The agent loop is working correctly. The LLM reasoning is logical. The problem is **insufficient input data** in the prompt. Fix the prompt, and the agent will succeed.
