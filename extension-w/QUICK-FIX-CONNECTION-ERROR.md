# Quick Fix: "Could not establish connection" Error

## The Problem

You're seeing this error:
```
Error: Could not establish connection. Receiving end does not exist.
```

## The Cause

You're trying to use the extension on a page where content scripts cannot run.

## The Solution

### ✅ Navigate to a Regular Website

**Won't Work On** ❌:
- `chrome://` pages (Settings, Extensions, etc.)
- `chrome-extension://` pages
- `edge://` pages
- `about:` pages
- Chrome Web Store
- PDF files (in some cases)
- Local `file://` pages

**Will Work On** ✅:
- https://www.google.com
- https://www.example.com
- https://www.amazon.com
- https://github.com
- Any regular http:// or https:// website

### Quick Test

1. Open a new tab
2. Go to: `https://www.google.com`
3. Click the Oryn extension icon
4. Try your command again

## What Changed

The extension now:
- ✅ Automatically checks if the page is valid
- ✅ Gives you a clear error message
- ✅ Tries to auto-inject content scripts if missing
- ✅ Tells you exactly what to do

## Example

**Before**:
```
❌ On chrome://extensions page
❌ Error: Could not establish connection. Receiving end does not exist.
❌ No idea what's wrong
```

**After (with fix)**:
```
❌ On chrome://extensions page
✅ Error: Cannot access this page. Please navigate to a regular website
✅ Clear explanation
✅ Auto-fix attempted
```

## Next Steps

1. **Reload the Extension**:
   ```
   chrome://extensions → Click refresh icon
   ```

2. **Navigate to a Test Site**:
   ```
   https://www.google.com
   ```

3. **Try Again**:
   - Open sidepanel
   - Enter command or task
   - Should work now!

## Still Not Working?

See `TROUBLESHOOTING.md` for:
- Complete troubleshooting guide
- Debug console commands
- How to report issues

---

**TL;DR**: Use the extension on regular websites (https://...), not on Chrome internal pages.
