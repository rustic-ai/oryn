# Oryn: Validation Use Cases

## Five Scenarios to Test and Validate the Solution

---

## Overview

These five use cases are designed to validate that Oryn's intent language approach provides meaningful advantages over screenshot-based, HTML-parsing, and function-call approaches. Each scenario exercises different system capabilities and presents challenges that expose the weaknesses of traditional methods.

### Validation Criteria

For each use case, we evaluate:

1. **Task Completion**: Can the agent accomplish the goal?
2. **Token Efficiency**: How much context is consumed?
3. **Error Recovery**: How does the agent handle unexpected states?
4. **Consistency**: Does behavior match across oryn-e, oryn-h, and oryn-r?
5. **Comparison**: How would screenshot/HTML/function approaches fare?

---

## Use Case 1: Research

### Scenario Description

An agent conducts multi-source research on a topic, gathering information from various websites, evaluating source credibility, extracting relevant data, and synthesizing findings into a coherent summary.

**Goal**: Research "the environmental impact of lithium mining for EV batteries" using at least 5 different sources, extract key statistics and claims, note source credibility, and identify areas of consensus and disagreement.

### Why This Tests Oryn

Research requires:
- Navigating search engines and following result links
- Distinguishing content from navigation/advertising
- Extracting specific data points from varied page layouts
- Managing multiple tabs or sequential page visits
- Handling diverse site structures (news, academic, corporate, government)

### Workflow Phases

**Phase 1: Search Initiation**

Agent begins with a search engine:

```
goto google.com
observe

@ google.com "Google"
[1] input/search "Search" {focused}
[2] button/submit "Google Search"
[3] button "I'm Feeling Lucky"

type 1 "environmental impact lithium mining EV batteries"
press Enter
```

**Expected Observation After Search**:
```
@ google.com/search "environmental impact lithium mining... - Google Search"

[1] input/search "environmental impact lithium mining EV batteries"
[2] link "Environmental Impacts of Lithium-Ion Batteries - EPA"
    snippet: "Lithium extraction requires significant water resources..."
[3] link "The Hidden Environmental Cost of EV Batteries - Reuters"
    snippet: "Mining operations in Chile and Australia..."
[4] link "Lithium Mining: Environmental Concerns - Nature.com"
    snippet: "Peer-reviewed analysis of ecosystem impacts..."
[5] link "Sustainable Lithium: Industry Perspectives - Tesla Blog"
    snippet: "Our commitment to responsible sourcing..."
...
[12] link "Next page"

# patterns
- search_results: [2,3,4,5,6,7,8,9,10,11]
- pagination: next=[12]
```

**Phase 2: Source Evaluation and Extraction**

Agent visits first source (EPA - government source, high credibility):

```
click 2
observe
```

**Expected Observation**:
```
@ epa.gov/lithium-batteries "Environmental Impacts of Lithium-Ion Batteries | EPA"

[1] nav "EPA Home"
[2] nav "Environmental Topics"
[3] heading "Environmental Impacts of Lithium-Ion Batteries"
[4] text-block "Key findings from EPA research..."
[5] link "Download Full Report (PDF)"
[6] link "Related: Battery Recycling Programs"
...

# patterns
- article_content: main=[3,4]
- navigation: breadcrumb=[1,2]
```

Agent extracts content:
```
text --selector "article"
```

**Response**:
```
Environmental Impacts of Lithium-Ion Batteries

Lithium extraction presents several environmental challenges:

Water Usage: Lithium brine extraction in South America's "Lithium Triangle" 
consumes approximately 2 million liters of water per ton of lithium produced.

Land Disruption: Hard-rock mining in Australia disturbs an average of 
1,500 tons of earth per ton of lithium extracted.

Chemical Contamination: Processing facilities have been linked to 
groundwater contamination in 23% of surveyed sites...
```

**Phase 3: Cross-Source Navigation**

Agent returns to search results and visits contrasting sources:

```
back
observe
click 5  # Tesla Blog - corporate source, potential bias
```

Agent continues through academic sources (Nature.com), news sources (Reuters), and additional perspectives, extracting key claims from each.

**Phase 4: Handling Varied Page Structures**

Different sites present different challenges:

- **News sites**: Cookie banners, subscription prompts, infinite scroll
- **Academic sites**: Paywalls, PDF downloads, citation formats
- **Corporate sites**: Marketing language, navigation complexity
- **Government sites**: Dense text, bureaucratic structure

Oryn pattern detection handles common obstacles:

```
@ reuters.com/article "The Hidden Environmental Cost..."

[1] modal/cookie "Accept Cookies"
[2] button "Accept All"
[3] button "Manage Preferences"
...

# patterns
- cookie_banner: accept=[2] reject=[3]
```

Agent dismisses and continues:
```
click 2
observe
```

**Phase 5: Synthesis**

After visiting 5+ sources, agent has extracted:
- Specific statistics (water usage, land disruption metrics)
- Source types (government, academic, corporate, news)
- Areas of agreement (water impact is significant)
- Areas of disagreement (whether impacts are manageable)

### Validation Points

| Criterion | What to Measure |
|-----------|-----------------|
| Task Completion | Did agent gather data from 5+ diverse sources? |
| Token Efficiency | Compare context used vs. HTML dump approach |
| Error Recovery | Did agent handle cookie banners, paywalls gracefully? |
| Consistency | Same results across oryn-e/h/r? |
| Comparison | Screenshot approach would struggle with text extraction; HTML approach would consume massive context |

### Why Traditional Approaches Fail Here

**Screenshot**: Cannot reliably extract specific statistics from varied page layouts. Would require multiple screenshots per page and expensive vision processing.

**HTML**: Each page dumps thousands of tokens. Five sources × ~10K tokens = 50K+ tokens just for raw content, leaving little room for reasoning.

**Function Calls**: Would require pre-defined schemas for each site type. Novel page structures would break extraction.

---

## Use Case 2: E-Commerce Purchase

### Scenario Description

An agent completes an end-to-end purchase: searching for a product, comparing options, selecting variants, adding to cart, and completing checkout with shipping and payment information.

**Goal**: Purchase a "men's blue oxford shirt, size large" from an online retailer, selecting the option with best value (considering price, reviews, and shipping time).

### Why This Tests Oryn

E-commerce involves:
- Product search with filters and sorting
- Complex product pages with variants (size, color)
- Dynamic cart interactions
- Multi-step checkout forms
- Session and authentication management
- Price and availability that changes dynamically

### Workflow Phases

**Phase 1: Product Search**

```
goto amazon.com
observe

@ amazon.com "Amazon.com"
[1] input/search "Search Amazon" {focused}
[2] button/submit "Go"
[3] link "Sign in"
...

type 1 "men's blue oxford shirt"
click 2
```

**Phase 2: Results Evaluation**

```
@ amazon.com/s "Amazon.com: men's blue oxford shirt"

[1] input/search "men's blue oxford shirt"
[2] select "Sort by" [Featured, Price: Low to High, Price: High to Low, Avg. Customer Review]
[3] checkbox "Prime" {unchecked}
[4] checkbox "Size: Large" {unchecked}
[5] product "Amazon Essentials Men's Oxford Shirt"
    price: $23.90
    rating: 4.3/5 (12,847 reviews)
    prime: yes
[6] product "Goodthreads Men's Standard-Fit Oxford"
    price: $29.00
    rating: 4.4/5 (8,234 reviews)
    prime: yes
[7] product "Van Heusen Men's Dress Shirt"
    price: $34.99
    rating: 4.1/5 (5,102 reviews)
    prime: yes
...

# patterns
- search_filters: [2,3,4]
- product_grid: [5,6,7,8,9,10]
```

Agent applies filters and evaluates:
```
check 4  # Size: Large
observe
click 5  # Best value option
```

**Phase 3: Product Page Interaction**

```
@ amazon.com/dp/B07... "Amazon Essentials Men's Oxford Shirt"

[1] heading "Amazon Essentials Men's Regular-Fit Long-Sleeve Oxford Shirt"
[2] select/color "Color" [Blue, White, Pink, Light Blue] {selected: Blue}
[3] select/size "Size" [S, M, L, XL, XXL] {selected: M}
[4] text "$23.90"
[5] text "In Stock"
[6] button/primary "Add to Cart" {primary}
[7] button "Buy Now"
[8] text "FREE delivery Thursday, January 16"
...

# patterns
- product_variants: color=[2] size=[3]
- purchase_actions: add_cart=[6] buy_now=[7]
```

Agent selects correct size:
```
select 3 "L"
click 6
```

**Phase 4: Cart and Checkout**

```
@ amazon.com/cart "Amazon Cart"

[1] heading "Shopping Cart"
[2] product "Amazon Essentials Men's Oxford Shirt - Blue - L"
    qty: 1
    price: $23.90
[3] select "Qty" [1,2,3,4,5,Delete]
[4] button "Delete"
[5] text "Subtotal: $23.90"
[6] button/primary "Proceed to checkout" {primary}
...

click 6
```

**Phase 5: Checkout Flow**

Multi-step form with shipping, payment, review:

```
@ amazon.com/checkout "Checkout"

[1] heading "Shipping Address"
[2] input/text "Full name" {required}
[3] input/text "Street address" {required}
[4] input/text "Apt, suite, unit" 
[5] input/text "City" {required}
[6] select "State" {required}
[7] input/text "ZIP Code" {required}
[8] input/tel "Phone number" {required}
[9] button/primary "Use this address" {primary}
...

# patterns
- shipping_form: [2,3,4,5,6,7,8] → [9]
```

Agent fills form fields:
```
type 2 "John Smith"
type 3 "123 Main Street"
type 5 "Seattle"
select 6 "WA"
type 7 "98101"
type 8 "206-555-0100"
click 9
```

### Validation Points

| Criterion | What to Measure |
|-----------|-----------------|
| Task Completion | Did agent complete purchase with correct item/size? |
| Token Efficiency | Context for variant selection vs. full HTML |
| Error Recovery | Handle out-of-stock, price changes, form errors |
| Consistency | Same checkout flow across modes |
| Comparison | Screenshot cannot read prices reliably; HTML misses dynamic state |

### Why Traditional Approaches Fail Here

**Screenshot**: Price text in images is unreliable. Variant selection requires understanding dropdown state. Cannot determine if button is disabled.

**HTML**: Product pages contain 50K+ tokens of markup. Variant selectors have complex JavaScript state not visible in static HTML.

**Function Calls**: Checkout forms vary per site. No universal schema for "buy product."

---

## Use Case 3: Travel Booking

### Scenario Description

An agent books a complete trip: searching flights, comparing options across criteria, selecting seats, and managing the booking through confirmation.

**Goal**: Book a round-trip flight from San Francisco (SFO) to New York (JFK), departing January 25, returning January 28, for one adult, preferring morning departure and window seat.

### Why This Tests Oryn

Travel booking involves:
- Complex date picker interactions
- Dynamic search results with many attributes
- Multi-step booking flows
- Seat selection with visual grid interfaces
- Session timeouts and price changes
- Form validation and error handling

### Workflow Phases

**Phase 1: Search Setup**

```
goto united.com
observe

@ united.com "United Airlines"
[1] radio "Round trip" {checked}
[2] radio "One way" {unchecked}
[3] input/text "From" {required}
[4] input/text "To" {required}
[5] input/date "Depart" {required}
[6] input/date "Return" {required}
[7] select "Travelers" {value: "1 Adult"}
[8] button/primary "Search" {primary}
...

# patterns
- flight_search: from=[3] to=[4] depart=[5] return=[6] submit=[8]
```

Agent fills search criteria:
```
type 3 "SFO"
type 4 "JFK"
```

**Phase 2: Date Picker Interaction**

Date pickers are notoriously complex:

```
click 5

@ united.com "United Airlines"
... (same elements)
[20] calendar "January 2025"
[21] button "Previous month"
[22] button "Next month"
[23] date "25" {available}
[24] date "26" {available}
[25] date "27" {available}
...

# patterns
- date_picker: month=[20] prev=[21] next=[22] dates=[23,24,25...]
```

Agent selects dates:
```
click 23  # January 25
click 6   # Return date field
click 28  # January 28 (element ID would be different)
click 8   # Search
```

**Phase 3: Flight Selection**

```
@ united.com/flights "Select Flights - United"

[1] heading "Select your departing flight: SFO → JFK, Sat Jan 25"
[2] flight "UA 123"
    depart: 6:45 AM → 3:22 PM
    duration: 5h 37m
    stops: Nonstop
    price: $342
[3] flight "UA 456"
    depart: 8:15 AM → 4:58 PM  
    duration: 5h 43m
    stops: Nonstop
    price: $298
[4] flight "UA 789"
    depart: 11:30 AM → 8:15 PM
    duration: 5h 45m
    stops: Nonstop
    price: $275
...
[10] button "Sort by" [Departure, Price, Duration]
[11] checkbox "Nonstop only" {checked}

# patterns  
- flight_results: [2,3,4,5,6,7,8,9]
- flight_filters: sort=[10] nonstop=[11]
```

Agent selects based on preferences (morning departure):
```
click 3  # 8:15 AM departure, good balance of time and price
```

**Phase 4: Seat Selection**

```
@ united.com/seats "Select Seats - United"

[1] heading "Select your seat"
[2] seat "12A" {type: window, status: available, price: +$0}
[3] seat "12B" {type: middle, status: available, price: +$0}
[4] seat "12C" {type: aisle, status: available, price: +$0}
[5] seat "12D" {type: aisle, status: occupied}
[6] seat "12E" {type: middle, status: available, price: +$0}
[7] seat "12F" {type: window, status: available, price: +$0}
...
[30] seat "6A" {type: window, status: available, price: +$45, class: extra-legroom}
...
[50] button/primary "Continue" {primary}

# patterns
- seat_map: rows=[2-49]
- seat_legend: window, middle, aisle, occupied, extra-legroom
```

Agent selects window seat:
```
click 2  # 12A - window seat
click 50 # Continue
```

**Phase 5: Passenger Information and Payment**

Similar to e-commerce checkout - form filling with validation.

### Validation Points

| Criterion | What to Measure |
|-----------|-----------------|
| Task Completion | Correct dates, preference matching, booking confirmed? |
| Token Efficiency | Date picker and seat map representation |
| Error Recovery | Handle sold-out flights, session timeout, price changes |
| Consistency | Complex interactions work across all modes |
| Comparison | Seat maps are nearly impossible with screenshots; HTML state is hidden |

### Why Traditional Approaches Fail Here

**Screenshot**: Seat maps are visual grids. Cannot determine seat status from image. Date pickers require precise coordinate clicking.

**HTML**: Calendar widgets have complex DOM. Seat availability is often in JavaScript state, not HTML attributes.

**Function Calls**: Date formats vary. Seat grid schemas differ per airline. No universal "book flight" function.

---

## Use Case 4: Account Management

### Scenario Description

An agent manages account settings across a complex web application: updating profile information, changing security settings, managing notification preferences, and reviewing account activity.

**Goal**: Log into a user's GitHub account, update the profile bio, enable two-factor authentication, configure email notification preferences to reduce noise, and review recent security events.

### Why This Tests Oryn

Account management involves:
- Authentication with existing sessions (oryn-r) or credentials
- Navigation through nested settings hierarchies
- Toggle switches and preference grids
- Security-sensitive operations
- Settings that span multiple pages/sections
- Confirmation dialogs and verification steps

### Workflow Phases

**Phase 1: Authentication**

Using oryn-r (Remote) mode to leverage existing session:

```
observe

@ github.com "GitHub"
[1] nav "Dashboard"
[2] avatar "user_avatar" 
[3] link "Your profile"
[4] link "Settings"
...
```

Agent is already authenticated via user's browser session.

Using oryn-h (Headless) would require login:
```
goto github.com/login
observe
login "username" "password"  # Intent command
```

**Phase 2: Profile Update**

```
click 4  # Settings

@ github.com/settings/profile "Profile Settings"
[1] nav "Profile"
[2] nav "Account" 
[3] nav "Appearance"
[4] nav "Notifications"
[5] nav "Security"
...
[10] input/text "Name" {value: "John Smith"}
[11] textarea "Bio" {value: "Software developer"}
[12] input/text "Company"
[13] input/text "Location"
[14] button/primary "Update profile" {primary}

# patterns
- settings_nav: [1,2,3,4,5,6,7,8]
- profile_form: [10,11,12,13] → [14]
```

Agent updates bio:
```
clear 11
type 11 "Software developer passionate about AI and open source. Building tools for the future."
click 14
```

**Response**:
```
ok click 14

# changes
~ [11] textarea "Bio" {value: "Software developer passionate about..."}
+ [20] alert/success "Profile updated successfully"
```

**Phase 3: Security Settings**

```
click 5  # Security nav

@ github.com/settings/security "Security Settings"
[1] heading "Two-factor authentication"
[2] text "Status: Disabled"
[3] button "Enable two-factor authentication"
[4] heading "Sessions"
[5] text "Active sessions: 3"
[6] link "View all sessions"
[7] heading "Security log"
[8] link "View security log"
...
```

Agent enables 2FA:
```
click 3

@ github.com/settings/two_factor "Enable Two-Factor Authentication"
[1] heading "Setup authenticator app"
[2] image/qr "Scan this QR code"
[3] text "Manual entry key: ABCD EFGH IJKL MNOP"
[4] input/text "Verify code from app" {required}
[5] button/primary "Enable" {primary, disabled}
...
```

At this point, agent would need to communicate with user (oryn-r scenario) or have access to TOTP generation.

**Phase 4: Notification Preferences**

```
back
click 4  # Notifications nav

@ github.com/settings/notifications "Notification Settings"
[1] heading "Email notifications"
[2] checkbox "Participating" {checked}
    description: "Notifications for threads you're participating in"
[3] checkbox "Watching" {checked}
    description: "Notifications for repositories you're watching"
[4] checkbox "Dependabot alerts" {checked}
[5] checkbox "Actions" {checked}
    description: "Notifications for GitHub Actions workflow runs"
...
[10] heading "Web notifications"
[11] checkbox "Enable web notifications" {checked}
...

# patterns
- notification_prefs: email=[2,3,4,5,6] web=[11,12,13]
```

Agent reduces notification noise:
```
uncheck 3  # Stop watching notifications
uncheck 5  # Stop Actions notifications
```

**Phase 5: Security Review**

```
back
click 5  # Security
click 8  # View security log

@ github.com/settings/security/audit-log "Security Log"
[1] heading "Security audit log"
[2] event "Sign in from new device"
    time: "2 hours ago"
    ip: "192.168.1.100"
    location: "Seattle, WA"
[3] event "Sign in"
    time: "Yesterday"
    ip: "192.168.1.100"
    location: "Seattle, WA"
[4] event "Repository access granted"
    time: "3 days ago"
    repo: "org/private-repo"
...
```

Agent reviews and reports any suspicious activity.

### Validation Points

| Criterion | What to Measure |
|-----------|-----------------|
| Task Completion | All settings changes applied correctly? |
| Token Efficiency | Navigation and toggle representation |
| Error Recovery | Handle permission errors, unsaved changes warnings |
| Consistency | Settings state accurate across modes |
| Comparison | Toggle states invisible in screenshots; checkbox HTML state varies |

### Why Traditional Approaches Fail Here

**Screenshot**: Cannot determine toggle/checkbox state from images. Security-sensitive operations need precise state awareness.

**HTML**: Settings pages use complex React/Vue components. State often in JavaScript, not HTML attributes.

**Function Calls**: Settings schemas are site-specific. No universal "change notification preference" function.

---

## Use Case 5: Content Publishing

### Scenario Description

An agent creates and publishes content on a platform: composing a post with rich formatting, adding media, configuring publishing options, and managing the published content.

**Goal**: Create a LinkedIn post announcing a new project, including formatted text, an image, relevant hashtags, and configured for optimal visibility. Then respond to the first comment received.

### Why This Tests Oryn

Content publishing involves:
- Rich text editors with formatting controls
- Media upload and management
- Hashtag and mention autocomplete
- Publishing options and visibility settings
- Real-time content updates (comments)
- Draft saving and preview functionality

### Workflow Phases

**Phase 1: Access Composer**

```
goto linkedin.com
observe

@ linkedin.com/feed "LinkedIn Feed"
[1] button "Start a post"
[2] input/search "Search"
[3] nav "Home"
[4] nav "My Network"
[5] nav "Jobs"
...
[20] post "Jane Doe shared a post..."
[21] post "Company X announced..."
...

click 1
```

**Phase 2: Compose Content**

```
@ linkedin.com/feed "LinkedIn Feed"
[1] modal "Create a post"
[2] textarea "What do you want to talk about?" {focused}
[3] button "Add a photo"
[4] button "Add a video"
[5] button "Add a document"
[6] button "Add a poll"
[7] select "Anyone" [Anyone, Connections only]
[8] button/primary "Post" {primary, disabled}
...

# patterns
- post_composer: content=[2] media=[3,4,5,6] visibility=[7] submit=[8]
```

Agent composes post:
```
type 2 "🚀 Excited to announce the launch of Oryn!

We've been working on a new approach to AI-browser interaction. Instead of forcing agents to parse screenshots or HTML, Oryn provides a semantic intent language designed for how AI actually thinks.

Key highlights:
• Three deployment modes for any environment
• Token-efficient observations
• Natural, forgiving command syntax

Check it out and let me know what you think!

#AI #Automation #OpenSource #Agents"
```

**Phase 3: Add Media**

```
click 3  # Add a photo

[10] modal "Add photo"
[11] button "Upload from computer"
[12] button "Choose from recent"
...

click 11
```

File picker interaction (backend-specific handling for upload).

After upload:
```
@ linkedin.com/feed "LinkedIn Feed"
...
[2] textarea "🚀 Excited to announce..." {value: "🚀 Excited..."}
[15] image "oryn-logo.png" {uploaded}
[16] button "Remove image"
[17] input/text "Add alt text"
[8] button/primary "Post" {primary, enabled}
...
```

Agent adds accessibility:
```
type 17 "Oryn logo - a stylized browser icon with three connected nodes"
```

**Phase 4: Configure and Publish**

```
select 7 "Anyone"  # Maximum visibility
click 8  # Post

ok click 8

# changes  
- [1] modal "Create a post"
@ linkedin.com/feed "LinkedIn Feed"
+ [25] post "Your post was published"
```

**Phase 5: Engagement Management**

After some time, agent checks for comments:

```
observe

@ linkedin.com/feed "LinkedIn Feed"
...
[30] post "You: 🚀 Excited to announce the launch of Oryn..."
    reactions: 47
    comments: 3
[31] button "Like"
[32] button "Comment"
[33] button "Share"
...

click 30  # View full post
```

```
@ linkedin.com/posts/... "Your Oryn Post"
[1] post "🚀 Excited to announce..."
[2] text "47 reactions"
[3] text "3 comments"
[4] comment "Alice: This looks amazing! How does it handle dynamic SPAs?"
[5] button "Reply" (to Alice)
[6] comment "Bob: Congrats on the launch! 🎉"
[7] button "Reply" (to Bob)
[8] comment "Carol: Interesting approach. How does it compare to Playwright?"
[9] button "Reply" (to Carol)
[10] textarea "Add a comment..."
...
```

Agent responds to first comment:
```
click 5  # Reply to Alice

[20] textarea "Reply to Alice..." {focused}
...

type 20 "Thanks Alice! Great question - Oryn's Universal Scanner handles SPAs beautifully because it runs inside the browser context, seeing the DOM exactly as rendered after JavaScript execution. The key is that we never parse HTML server-side."
press Enter
```

### Validation Points

| Criterion | What to Measure |
|-----------|-----------------|
| Task Completion | Post published with all elements? Comment reply successful? |
| Token Efficiency | Rich editor state representation |
| Error Recovery | Handle upload failures, character limits, draft auto-save |
| Consistency | Media handling across modes |
| Comparison | Rich text state impossible in screenshots; editor HTML is complex |

### Why Traditional Approaches Fail Here

**Screenshot**: Cannot read own composed text. Cannot see character counts or validation state. Image upload status unclear.

**HTML**: Rich text editors (Draft.js, Quill, ProseMirror) have complex DOM structures that don't match visual output.

**Function Calls**: Every platform has different compose schemas. No universal "publish post with image" function.

---

## Cross-Scenario Validation Matrix

| Capability | Research | E-Commerce | Travel | Account | Content |
|------------|----------|------------|--------|---------|---------|
| Navigation | ✓ | ✓ | ✓ | ✓ | ✓ |
| Form Filling | | ✓ | ✓ | ✓ | |
| Text Extraction | ✓ | | | | |
| Pattern Detection | ✓ | ✓ | ✓ | ✓ | ✓ |
| Dynamic Content | ✓ | ✓ | ✓ | | ✓ |
| Variant Selection | | ✓ | ✓ | | |
| Date Pickers | | | ✓ | | |
| File Upload | | | | | ✓ |
| Grid Interfaces | | | ✓ | | |
| Toggle/Checkbox | | ✓ | | ✓ | |
| Real-time Updates | | ✓ | ✓ | | ✓ |
| Multi-step Flow | | ✓ | ✓ | ✓ | |
| Cookie/Modal Handling | ✓ | ✓ | ✓ | | |
| Authentication | | ✓ | ✓ | ✓ | ✓ |

---

## Success Criteria Summary

For Oryn validation to be considered successful:

1. **All five use cases complete end-to-end** without requiring fallback to screenshots, raw HTML, or custom function definitions.

2. **Token consumption is measurably lower** than equivalent HTML-dump approaches (target: 80% reduction in context usage).

3. **Error recovery is automatic** for common obstacles (cookie banners, modals, stale elements) without agent-side special handling.

4. **Behavior is identical** across oryn-e, oryn-h, and oryn-r for the same page states.

5. **Agent development time is reduced** compared to building equivalent functionality with existing tools (target: 50% faster implementation).

---

*Document Version: 1.0*  
*Last Updated: January 2025*
