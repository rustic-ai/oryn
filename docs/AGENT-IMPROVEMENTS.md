# Agent System Improvements

## Overview

Based on the Google Store agent execution analysis, four critical improvements are needed:

1. **Inform agents about element limits** - Agent needs to know what it can/can't see
2. **OIL should suggest next actions** - Proactive guidance on what to do next
3. **Better pattern detection** - Recognize more UI patterns beyond basic forms
4. **Generic element filtering** - Smart, context-aware element prioritization

---

## 1. Inform Agent About Element Limits

### Problem

Agent sees:
```
[WASM] Scan has 200 elements
[WASM] Element 0: button "Skip navigation"
[WASM] Element 1: div "Phones\nEarbuds..."
...only first 5 shown in logs
```

But doesn't know:
- Why it only sees 5 elements in the prompt
- How to request elements 6-200
- What filtering is applied
- That product links exist in elements 50-150

### Solution A: Add Scan Summary Metadata

**Extend `ScanResult` to include summary:**

```rust
// crates/oryn-common/src/protocol.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub page: PageInfo,
    pub elements: Vec<Element>,
    pub stats: ScanStats,

    // NEW: Summary of what's in the scan
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ScanSummary>,

    // ... existing fields ...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    /// Total elements scanned
    pub total_elements: usize,

    /// How many included in this result
    pub included_elements: usize,

    /// Breakdown by element type
    pub element_types: HashMap<String, usize>,
    // e.g., {"link": 45, "button": 12, "input": 5}

    /// Suggested commands to see more
    pub pagination_hints: Vec<String>,
    // e.g., ["observe full", "observe from 50", "scroll down"]

    /// What filters were applied
    pub filters_applied: Vec<String>,
    // e.g., ["viewport_only", "interactive_only"]
}
```

**Scanner.js generates summary:**

```javascript
// crates/oryn-scanner/src/scanner.js (in scan function)

const summary = {
    total_elements: elements.length,
    included_elements: elementsToReturn.length,
    element_types: {},
    pagination_hints: [],
    filters_applied: []
};

// Count element types
for (const el of elements) {
    const type = el.type + (el.role ? `/${el.role}` : '');
    summary.element_types[type] = (summary.element_types[type] || 0) + 1;
}

// Add hints if truncated
if (elementsToReturn.length < elements.length) {
    summary.pagination_hints.push('observe full');
    summary.pagination_hints.push(`observe from ${elementsToReturn.length}`);
}

// Document filters
if (params.viewport_only) summary.filters_applied.push('viewport_only');
if (params.max_elements) summary.filters_applied.push(`max_elements: ${params.max_elements}`);
```

### Solution B: Update Formatter to Show Summary

**Formatter shows metadata at the top:**

```rust
// crates/oryn-common/src/formatter.rs

pub fn format_response(resp: &ScannerProtocolResponse) -> String {
    match resp {
        ScannerProtocolResponse::Ok { data, .. } => match data.as_ref() {
            ScannerData::Scan(scan) => {
                let mut output = String::new();

                // Add summary if available
                if let Some(summary) = &scan.summary {
                    output.push_str(&format!(
                        "# Showing {} of {} elements\n",
                        summary.included_elements,
                        summary.total_elements
                    ));

                    if !summary.element_types.is_empty() {
                        output.push_str("# Element types: ");
                        let types: Vec<String> = summary.element_types.iter()
                            .map(|(k, v)| format!("{}: {}", k, v))
                            .collect();
                        output.push_str(&types.join(", "));
                        output.push_str("\n");
                    }

                    if !summary.pagination_hints.is_empty() {
                        output.push_str("# To see more: ");
                        output.push_str(&summary.pagination_hints.join(" or "));
                        output.push_str("\n");
                    }

                    output.push_str("\n");
                }

                // ... existing element formatting ...
            }
        }
    }
}
```

**Example output:**

```
# Showing 50 of 200 elements
# Element types: link: 45, button: 12, input: 8, div: 135
# To see more: observe full or observe from 50

@ https://store.google.com/phones "Google Store - Phones"
[1] button "Skip navigation"
[2] link "Phones"
...
[50] link "Google Pixel 9 Pro"
```

---

## 2. OIL Should Suggest Next Actions

### Problem

After clicking "Phones", agent doesn't know:
- Products are now visible → should extract
- Can scroll to see more → should scroll
- Has a search box → can refine

OIL is passive - just reports what happened. Should be proactive.

### Solution A: Use Existing `available_intents`

The `ScanResult` already has `available_intents: Option<Vec<IntentAvailability>>` but it's not populated!

**Populate intents in scanner.js:**

```javascript
// crates/oryn-scanner/src/scanner.js

function suggestNextActions(scan) {
    const suggestions = [];

    // Detect product listings
    const productLinks = scan.elements.filter(el =>
        el.type === 'link' &&
        (el.text?.includes('$') || el.text?.includes('Price') || el.text?.match(/\d+\.\d{2}/))
    );

    if (productLinks.length > 5) {
        suggestions.push({
            name: 'extract_products',
            status: 'Ready',
            parameters: ['css selector or element IDs'],
            trigger_reason: `Found ${productLinks.length} product links`
        });
    }

    // Detect search capability
    if (scan.patterns?.search) {
        suggestions.push({
            name: 'search',
            status: 'Ready',
            parameters: ['query text'],
            trigger_reason: 'Search box detected'
        });
    }

    // Detect pagination
    if (scan.patterns?.pagination) {
        suggestions.push({
            name: 'next_page',
            status: 'Ready',
            parameters: [],
            trigger_reason: 'Pagination controls detected'
        });
    }

    // Detect scrollability
    if (scan.page.scroll.max_y > 0) {
        suggestions.push({
            name: 'scroll_down',
            status: 'Ready',
            parameters: [],
            trigger_reason: 'More content below (scroll to see)'
        });
    }

    // Detect forms
    if (scan.patterns?.login) {
        suggestions.push({
            name: 'login',
            status: 'Ready',
            parameters: ['email/username', 'password'],
            trigger_reason: 'Login form detected'
        });
    }

    return suggestions.length > 0 ? suggestions : null;
}

// Add to scan response
response.available_intents = suggestNextActions(response);
```

### Solution B: Format Suggestions in Output

**Show suggestions after patterns:**

```rust
// crates/oryn-common/src/formatter.rs

// After patterns section:
if let Some(intents) = &scan.available_intents {
    if !intents.is_empty() {
        output.push_str("\n\nSuggested Actions:");
        for intent in intents {
            output.push_str(&format!("\n- {}", intent.name));
            if let Some(reason) = &intent.trigger_reason {
                output.push_str(&format!(" ({})", reason));
            }
            if !intent.parameters.is_empty() {
                output.push_str(&format!(" [{}]", intent.parameters.join(", ")));
            }
        }
    }
}
```

**Example output:**

```
@ https://store.google.com/phones "Google Store - Phones"
[1] button "Skip navigation"
...
[45] link "Google Pixel 9 Pro - $999"
[46] link "Google Pixel 8a - $499"

Patterns:
- Search Box
- Pagination

Suggested Actions:
- extract_products (Found 15 product links) [css selector or element IDs]
- scroll_down (More content below)
- next_page (Pagination controls detected)
```

### Solution C: Add to Agent Prompt

**Ralph Agent includes suggestions in prompt:**

```javascript
// extension-w/agent/prompts.js

export const RALPH_SYSTEM_PROMPT = `You are a web automation agent using OIL.

CURRENT PAGE:
{scan_output}

${suggestions ? `
AVAILABLE ACTIONS (recommended):
${suggestions.map(s => `- ${s.name}: ${s.trigger_reason}`).join('\n')}
` : ''}

Based on the current page and suggested actions, decide the next OIL command.
`;
```

---

## 3. Better Pattern Detection

### Problem

Current patterns: `login`, `search`, `pagination`, `modal`, `cookie_banner`

Missing patterns:
- **Product listings** (e-commerce cards)
- **Content articles** (blog posts, news)
- **Navigation** (breadcrumbs, menus)
- **Data tables**
- **Media galleries**
- **User profiles**
- **Comments/reviews**
- **Filters/facets**

### Solution A: Add New Pattern Types

```rust
// crates/oryn-common/src/protocol.rs

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetectedPatterns {
    // Existing
    pub login: Option<LoginPattern>,
    pub search: Option<SearchPattern>,
    pub pagination: Option<PaginationPattern>,
    pub modal: Option<ModalPattern>,
    pub cookie_banner: Option<CookieBannerPattern>,

    // NEW PATTERNS
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_listing: Option<ProductListingPattern>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationPattern>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_table: Option<DataTablePattern>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub article: Option<ArticlePattern>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<FiltersPattern>,
}

// Product Listing Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductListingPattern {
    /// IDs of detected product cards
    pub product_cards: Vec<u32>,

    /// Common structure: image, title, price, button
    pub card_structure: CardStructure,

    /// Confidence that these are products
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardStructure {
    pub has_images: bool,
    pub has_prices: bool,
    pub has_titles: bool,
    pub has_cta_buttons: bool,
}

// Navigation Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationPattern {
    pub breadcrumbs: Option<Vec<u32>>,
    pub main_menu: Option<Vec<u32>>,
    pub category_links: Vec<u32>,
}

// Data Table Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataTablePattern {
    pub table_id: u32,
    pub headers: Vec<String>,
    pub row_count: usize,
    pub sortable: bool,
    pub filterable: bool,
}

// Article Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticlePattern {
    pub title: Option<u32>,
    pub author: Option<String>,
    pub publish_date: Option<String>,
    pub content_blocks: Vec<u32>,
    pub word_count: Option<usize>,
}

// Filters Pattern (e-commerce facets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiltersPattern {
    pub price_range: Option<u32>,
    pub category_filters: Vec<u32>,
    pub sort_dropdown: Option<u32>,
}
```

### Solution B: Implement Product Listing Detector

```javascript
// crates/oryn-scanner/src/scanner.js

const Patterns = {
    // ... existing patterns ...

    detectProductListing: (elements) => {
        // Look for repeated card-like structures
        const cards = [];
        const potentialProducts = elements.filter(el =>
            el.type === 'div' || el.type === 'article' || el.type === 'section'
        );

        for (const el of potentialProducts) {
            // Check if element contains product indicators
            const text = (el.text || '').toLowerCase();
            const hasPrice = text.match(/\$\d+|\d+\.\d{2}|price/);
            const hasImage = el.children?.some(c => c.type === 'img');
            const hasButton = el.children?.some(c =>
                c.type === 'button' &&
                (c.text?.includes('Add') || c.text?.includes('Buy'))
            );

            if (hasPrice || (hasImage && hasButton)) {
                cards.push(el.id);
            }
        }

        // Need at least 3 similar cards to be a listing
        if (cards.length < 3) return null;

        // Calculate confidence
        const avgSimilarity = calculateCardSimilarity(cards, elements);
        const confidence = Math.min(avgSimilarity * 0.7 + (cards.length / 20) * 0.3, 1.0);

        return {
            product_cards: cards,
            card_structure: {
                has_images: true,
                has_prices: true,
                has_titles: true,
                has_cta_buttons: true
            },
            confidence: confidence
        };
    },

    detectNavigation: (elements) => {
        const breadcrumbs = elements.filter(el =>
            (el.attributes?.['aria-label']?.includes('breadcrumb') ||
             el.attributes?.class?.includes('breadcrumb')) &&
            el.type === 'nav'
        ).map(el => el.id);

        const categoryLinks = elements.filter(el =>
            el.type === 'link' &&
            (el.role === 'navigation' || el.attributes?.class?.includes('category'))
        ).map(el => el.id);

        if (breadcrumbs.length === 0 && categoryLinks.length === 0) {
            return null;
        }

        return {
            breadcrumbs: breadcrumbs.length > 0 ? breadcrumbs : null,
            category_links: categoryLinks
        };
    },

    // Add to detectAll:
    detectAll: (elements) => {
        const detectors = [
            ['login', Patterns.detectLogin],
            ['search', Patterns.detectSearch],
            ['pagination', Patterns.detectPagination],
            ['modal', Patterns.detectModal],
            ['cookie_banner', Patterns.detectCookieBanner],
            ['product_listing', Patterns.detectProductListing],  // NEW
            ['navigation', Patterns.detectNavigation],            // NEW
        ];

        const patterns = {};
        for (const [name, detector] of detectors) {
            const result = detector(elements);
            if (result) patterns[name] = result;
        }

        return Object.keys(patterns).length > 0 ? patterns : null;
    }
};
```

---

## 4. Generic Element Filtering

### Problem

200 elements is too many for LLM context, but we need the right ones:
- Agent looking for products → show product links
- Agent looking for forms → show inputs/buttons
- Agent extracting content → show text blocks

Current filtering is static (first N elements). Need dynamic, context-aware filtering.

### Solution A: Multi-Stage Filtering

**Stage 1: Remove Noise**
```javascript
function removeNoise(elements) {
    return elements.filter(el => {
        // Remove elements with no useful content
        if (!el.text && !el.label && el.type !== 'input') return false;

        // Remove tiny/hidden elements
        if (el.rect.width < 10 || el.rect.height < 10) return false;

        // Remove pure layout elements
        if (el.type === 'div' && !el.text && !el.children?.length) return false;

        // Remove spacers/separators
        if (el.text?.trim() === '' || el.text?.match(/^[-\s|]+$/)) return false;

        return true;
    });
}
```

**Stage 2: Prioritize by Type**
```javascript
function prioritizeElements(elements, context = {}) {
    const scores = new Map();

    for (const el of elements) {
        let score = 0;

        // Base score by type
        if (el.type === 'button') score += 10;
        if (el.type === 'link') score += 8;
        if (el.type === 'input') score += 9;
        if (el.role === 'primary') score += 5;

        // Context-based scoring
        if (context.lookingForProducts) {
            if (el.text?.match(/\$\d+|\d+\.\d{2}/)) score += 15;
            if (el.type === 'img') score += 5;
            if (el.text?.toLowerCase().includes('buy')) score += 10;
        }

        if (context.lookingForForms) {
            if (el.type === 'input') score += 15;
            if (el.role === 'submit') score += 10;
        }

        if (context.lookingForContent) {
            if (el.type === 'article' || el.type === 'p') score += 10;
            if (el.text && el.text.length > 100) score += 5;
        }

        // Boost elements in viewport
        if (isInViewport(el)) score += 3;

        scores.set(el.id, score);
    }

    // Sort by score
    return elements.sort((a, b) =>
        (scores.get(b.id) || 0) - (scores.get(a.id) || 0)
    );
}
```

**Stage 3: Smart Truncation**
```javascript
function smartTruncate(elements, maxElements = 50, context = {}) {
    const prioritized = prioritizeElements(elements, context);

    // Always include high-priority elements
    const highPriority = prioritized.filter(el =>
        el.type === 'button' ||
        el.type === 'input' ||
        (el.type === 'link' && el.text && el.text.length > 0)
    ).slice(0, maxElements * 0.6);  // 60% high priority

    // Fill remaining with next-best elements
    const remaining = prioritized
        .filter(el => !highPriority.includes(el))
        .slice(0, maxElements - highPriority.length);

    return [...highPriority, ...remaining];
}
```

### Solution B: Pattern-Aware Filtering

When a pattern is detected, filter elements relevant to that pattern:

```javascript
function filterByPattern(elements, patterns) {
    if (!patterns) return elements;

    const relevant = new Set();

    // If product listing detected, prioritize product elements
    if (patterns.product_listing) {
        patterns.product_listing.product_cards.forEach(id => relevant.add(id));
    }

    // If login form detected, prioritize form elements
    if (patterns.login) {
        if (patterns.login.email) relevant.add(patterns.login.email);
        if (patterns.login.username) relevant.add(patterns.login.username);
        if (patterns.login.password) relevant.add(patterns.login.password);
        if (patterns.login.submit) relevant.add(patterns.login.submit);
    }

    // If pagination detected, include pagination controls
    if (patterns.pagination) {
        if (patterns.pagination.prev) relevant.add(patterns.pagination.prev);
        if (patterns.pagination.next) relevant.add(patterns.pagination.next);
        patterns.pagination.pages?.forEach(id => relevant.add(id));
    }

    // Keep all relevant + top N others
    const filtered = elements.filter(el => relevant.has(el.id));
    const others = elements.filter(el => !relevant.has(el.id)).slice(0, 30);

    return [...filtered, ...others];
}
```

### Solution C: Configurable Filtering in OIL

Add OIL commands to control filtering:

```
observe                    # Default: smart filtering
observe full               # All elements, no filtering
observe interactive        # Only buttons, links, inputs
observe products           # Focus on product cards
observe forms              # Focus on form elements
observe content            # Focus on text blocks
observe near "search box"  # Elements near a specific element
```

---

## Implementation Priority

### Phase 1: Quick Wins (1-2 days)
1. ✅ Add scan summary to formatter output
2. ✅ Implement basic element filtering (remove noise)
3. ✅ Populate `available_intents` with basic suggestions

### Phase 2: Pattern Detection (2-3 days)
1. Add ProductListingPattern type to protocol
2. Implement detectProductListing in scanner.js
3. Add NavigationPattern and detector
4. Update formatter to show new patterns

### Phase 3: Smart Filtering (2-3 days)
1. Implement multi-stage filtering
2. Add context-aware scoring
3. Add pattern-aware filtering
4. Add OIL commands for filter control

### Phase 4: Agent Integration (1-2 days)
1. Update Ralph Agent to use scan summary
2. Include available_intents in agent prompt
3. Add pattern-based decision logic
4. Test on real e-commerce sites

---

## Expected Improvements

**Before:**
```
Agent sees:
[1] button "Skip navigation"
[2] div "Phones\nEarbuds..."
[3] link "Google Store"
```

**After:**
```
# Showing 50 of 200 elements (interactive_only filter)
# Types: link:45, button:12, input:3
# To see more: observe full or observe products

@ https://store.google.com/phones
[1] link "Google Pixel 9 Pro - $999"
[2] link "Google Pixel 8a - $499"
[3] button "Add to Cart"
...

Patterns:
- Product Listing (15 products, 95% confidence)
- Search Box
- Pagination

Suggested Actions:
- extract_products (Found 15 product links)
- search (Refine product search)
- next_page (View more products)
```

Agent now understands:
- ✅ What it can see (50 of 200)
- ✅ How to see more (observe full)
- ✅ What's on the page (products, not just navigation)
- ✅ What to do next (extract, search, paginate)

---

## Questions for Discussion

1. **Filtering Strategy**: Should we default to aggressive filtering (50 elements) or show more (100+) with summary?

2. **Pattern Confidence**: What confidence threshold should trigger pattern-based suggestions? (Currently 0.7)

3. **LLM Context Limits**: Chrome AI has ~2K token context. How many elements can we realistically show?

4. **Extraction Commands**: Should we add new OIL commands like `extract products` or keep it generic with `extract text from links`?

5. **Pattern Detection Accuracy**: ML-based detection vs. heuristics? Trade-offs?
