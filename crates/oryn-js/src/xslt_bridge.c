/* Browser XSLT 1.0 boundary. No libxslt resource access is permitted. */
#include <libxml/parser.h>
#include <libxml/xmlmemory.h>
#include <libxml/HTMLtree.h>
#include <libxml/xmlIO.h>
#include <stdint.h>
#include <libxml/xpathInternals.h>
#include <libxslt/xslt.h>
#include <libxslt/xsltInternals.h>
#include <libxslt/transform.h>
#include <libxslt/xsltutils.h>
#include <libxslt/documents.h>
#include <libxslt/variables.h>
#include <libxslt/security.h>
#ifdef _WIN32
#include <windows.h>
#define ORYN_THREAD_LOCAL __declspec(thread)
#else
#include <pthread.h>
#define ORYN_THREAD_LOCAL _Thread_local
#endif
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

typedef struct { const char *namespace_uri; const char *local; const char *value; } OrynXsltParameter;
typedef struct { unsigned char *bytes; size_t length; int kind; } OrynXsltOutput;
typedef struct { const char *url; const char *base; const unsigned char *bytes; size_t length; } OrynXsltResource;
static ORYN_THREAD_LOCAL const OrynXsltResource *resources;
static ORYN_THREAD_LOCAL size_t resource_count;
static xmlDocPtr parse_at(const unsigned char *, size_t, const char *);
#ifdef _WIN32
static INIT_ONCE initialized = INIT_ONCE_STATIC_INIT;
#else
static pthread_once_t initialized = PTHREAD_ONCE_INIT;
#endif
static xmlDocPtr deny_load(const xmlChar *uri, xmlDictPtr dict, int options, void *context, xsltLoadType type) {
    (void)dict; (void)options; (void)context;
    if (type != XSLT_LOAD_STYLESHEET) return NULL;
    for (size_t i = 0; i < resource_count; ++i)
        if (xmlStrEqual(uri, BAD_CAST resources[i].url))
            return parse_at(resources[i].bytes, resources[i].length, resources[i].base);
    return NULL;
}
/* Compatible allocator interposition: pointers retain the original allocator's
 * representation. Untracked allocations (including pre-initialization users)
 * always go to their original free/realloc functions. No allocation header is
 * ever read from a foreign pointer. Only this thread's active operation accrues
 * a budget; tracked allocations retain it until their actual native free. */
typedef struct OrynBudget { size_t live; size_t refs; int failed; } OrynBudget;
typedef struct OrynAllocation { void *pointer; size_t size; OrynBudget *budget; struct OrynAllocation *next; } OrynAllocation;
#define ORYN_ALLOCATION_BUCKETS 4096
#define ORYN_NATIVE_LIMIT (64U * 1024U * 1024U)
static OrynAllocation *allocations[ORYN_ALLOCATION_BUCKETS];
static ORYN_THREAD_LOCAL OrynBudget *active_budget;
static xmlFreeFunc original_free;
static xmlMallocFunc original_malloc;
static xmlReallocFunc original_realloc;
static xmlStrdupFunc original_strdup;
#ifdef _WIN32
static SRWLOCK allocation_lock = SRWLOCK_INIT;
#define LOCK_ALLOCATIONS() AcquireSRWLockExclusive(&allocation_lock)
#define UNLOCK_ALLOCATIONS() ReleaseSRWLockExclusive(&allocation_lock)
#else
static pthread_mutex_t allocation_lock = PTHREAD_MUTEX_INITIALIZER;
#define LOCK_ALLOCATIONS() pthread_mutex_lock(&allocation_lock)
#define UNLOCK_ALLOCATIONS() pthread_mutex_unlock(&allocation_lock)
#endif
static size_t bucket(void *pointer) { return ((size_t)(uintptr_t)pointer >> 4) % ORYN_ALLOCATION_BUCKETS; }
static OrynAllocation **find_allocation(void *pointer) {
    OrynAllocation **entry = &allocations[bucket(pointer)];
    while (*entry && (*entry)->pointer != pointer) entry = &(*entry)->next;
    return entry;
}
static void release_budget(OrynBudget *budget) { if (--budget->refs == 0) free(budget); }
static int reserve_budget(OrynBudget *budget, size_t amount) {
    if (budget->failed || amount > ORYN_NATIVE_LIMIT || budget->live > ORYN_NATIVE_LIMIT - amount) {
        budget->failed = 1; return 0;
    }
    budget->live += amount; return 1;
}
static void *limited_malloc(size_t size) {
    OrynBudget *budget = active_budget;
    if (!budget) return original_malloc(size);
    LOCK_ALLOCATIONS();
    if (size > SIZE_MAX - sizeof(OrynAllocation) || !reserve_budget(budget, size + sizeof(OrynAllocation))) {
        budget->failed = 1; UNLOCK_ALLOCATIONS(); return NULL;
    }
    UNLOCK_ALLOCATIONS();
    void *pointer = original_malloc(size);
    OrynAllocation *entry = pointer ? malloc(sizeof(*entry)) : NULL;
    LOCK_ALLOCATIONS();
    if (!entry) { budget->live -= size + sizeof(OrynAllocation); budget->failed = 1; }
    else {
        *entry = (OrynAllocation){pointer, size, budget, allocations[bucket(pointer)]};
        allocations[bucket(pointer)] = entry; ++budget->refs;
    }
    UNLOCK_ALLOCATIONS();
    if (!entry && pointer) original_free(pointer);
    return entry ? pointer : NULL;
}
static void limited_free(void *pointer) {
    if (!pointer) return;
    LOCK_ALLOCATIONS();
    OrynAllocation **slot = find_allocation(pointer), *entry = *slot;
    if (entry) {
        *slot = entry->next;
        entry->budget->live -= entry->size + sizeof(*entry);
        release_budget(entry->budget); free(entry);
    }
    UNLOCK_ALLOCATIONS();
    original_free(pointer);
}
static void *limited_realloc(void *pointer, size_t size) {
    if (!pointer) return limited_malloc(size);
    if (!size) { limited_free(pointer); return NULL; }
    LOCK_ALLOCATIONS();
    OrynAllocation **slot = find_allocation(pointer), *entry = *slot;
    OrynBudget *budget = entry ? entry->budget : active_budget;
    if (!budget) { UNLOCK_ALLOCATIONS(); return original_realloc(pointer, size); }
    size_t old = entry ? entry->size : 0;
    size_t overhead = entry ? 0 : sizeof(OrynAllocation);
    if (size > SIZE_MAX - overhead || (size > old && !reserve_budget(budget, size - old + overhead))) {
        budget->failed = 1; UNLOCK_ALLOCATIONS(); return NULL;
    }
    OrynAllocation *fresh = entry ? entry : malloc(sizeof(*fresh));
    if (!fresh) {
        if (size > old) budget->live -= size - old + overhead;
        budget->failed = 1; UNLOCK_ALLOCATIONS(); return NULL;
    }
    /* Original realloc cannot reenter these hooks; holding the map lock keeps
     * pointer movement and its accounting atomic to frees on other threads. */
    void *moved = original_realloc(pointer, size);
    if (!moved) {
        if (size > old) budget->live -= size - old + overhead;
        budget->failed = 1; if (!entry) free(fresh);
        UNLOCK_ALLOCATIONS(); return NULL;
    }
    if (entry) *slot = entry->next;
    else ++budget->refs;
    if (size < old) budget->live -= old - size;
    *fresh = (OrynAllocation){moved, size, budget, allocations[bucket(moved)]};
    allocations[bucket(moved)] = fresh;
    UNLOCK_ALLOCATIONS(); return moved;
}
static char *limited_strdup(const char *source) {
    if (!source) return NULL;
    size_t length = strlen(source);
    if (length == SIZE_MAX) return NULL;
    char *result = limited_malloc(length + 1);
    if (result) memcpy(result, source, length + 1);
    return result;
}
static int begin_budget(void) {
    xmlFreeFunc f; xmlMallocFunc m; xmlReallocFunc r; xmlStrdupFunc s;
    if (active_budget || xmlMemGet(&f, &m, &r, &s) != 0 || f != limited_free || m != limited_malloc || r != limited_realloc || s != limited_strdup) return 0;
    active_budget = calloc(1, sizeof(*active_budget));
    if (!active_budget) return 0;
    active_budget->refs = 1; return 1;
}
static int end_budget(void) {
    OrynBudget *budget = active_budget; active_budget = NULL;
    LOCK_ALLOCATIONS();
    int success = !budget->failed; release_budget(budget);
    UNLOCK_ALLOCATIONS(); return success;
}
static void initialize(void) {
    if (xmlMemGet(&original_free, &original_malloc, &original_realloc, &original_strdup) != 0) return;
    if (original_free != free || original_malloc != malloc || original_realloc != realloc) return;
    if (xmlMemSetup(limited_free, limited_malloc, limited_realloc, limited_strdup) != 0) return;
    xmlInitParser();
    xsltInit();
    /* This browser owns all libxslt use. Never restore an I/O-capable loader. */
    xsltSetLoaderFunc(deny_load);
}
#ifdef _WIN32
static BOOL CALLBACK initialize_windows(PINIT_ONCE once, PVOID parameter, PVOID *context) {
    (void)once; (void)parameter; (void)context; initialize(); return TRUE;
}
static void initialize_once(void) { InitOnceExecuteOnce(&initialized, initialize_windows, NULL, NULL); }
#else
static void initialize_once(void) { pthread_once(&initialized, initialize); }
#endif
static void empty_document(xmlXPathParserContextPtr context, int count) {
    for (int i = 0; i < count; ++i) xmlXPathFreeObject(valuePop(context));
    valuePush(context, xmlXPathNewNodeSet(NULL));
}
static xmlDocPtr parse_at(const unsigned char *data, size_t length, const char *url) {
    if (!data || length == 0 || length > 16 * 1024 * 1024) return NULL;
    for (size_t i = 0; i < length; ++i) {
        const char *end = NULL; size_t open = 0, close = 0;
        if (i + 4 <= length && memcmp(data + i, "<!--", 4) == 0) { end = "-->"; open = 4; close = 3; }
        else if (i + 9 <= length && memcmp(data + i, "<![CDATA[", 9) == 0) { end = "]]>"; open = 9; close = 3; }
        else if (i + 2 <= length && memcmp(data + i, "<?", 2) == 0) { end = "?>"; open = 2; close = 2; }
        if (end) {
            size_t j = i + open;
            while (j + close <= length && memcmp(data + j, end, close) != 0) ++j;
            if (j + close > length) return NULL;
            i = j + close - 1; continue;
        }
        if (i + 9 > length || memcmp(data + i, "<!DOCTYPE", 9) != 0) continue;
        unsigned char quote = 0;
        for (size_t j = i + 9; j < length; ++j) {
            unsigned char c = data[j];
            if (quote) { if (c == quote) quote = 0; continue; }
            if (c == '\'' || c == '"') { quote = c; continue; }
            if (c == '[') return NULL;
            if (c == '>') { i = j; break; }
        }
    }
    xmlDocPtr doc = xmlReadMemory((const char *)data, (int)length, url, "UTF-8",
                                 XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_HUGE);
    /* No DTDLOAD/NOENT; the forbidden internal subset was rejected above. */
    return doc;
}
static xsltStylesheetPtr compile(const unsigned char *data, size_t length, const char *url) {
    xmlDocPtr doc = parse_at(data, length, url);
    if (!doc) return NULL;
    xsltStylesheetPtr sheet = xsltParseStylesheetDoc(doc);
    if (!sheet) { xmlFreeDoc(doc); return NULL; }
    if (sheet->errors) { xsltFreeStylesheet(sheet); return NULL; }
    return sheet;
}
int oryn_xslt_validate(const unsigned char *style, size_t length, const char *url, const OrynXsltResource *loaded, size_t loaded_count) {
    initialize_once();
    if (!begin_budget()) return 0;
    resources = loaded; resource_count = loaded_count;
    xsltStylesheetPtr sheet = compile(style, length, url);
    resources = NULL; resource_count = 0;
    int valid = sheet != NULL;
    if (sheet) xsltFreeStylesheet(sheet);
    return end_budget() && valid;
}
int oryn_xslt_transform(const unsigned char *style, size_t style_length,
                        const unsigned char *source, size_t source_length,
                        const OrynXsltParameter *parameters, size_t count, const char *style_url, const OrynXsltResource *loaded, size_t loaded_count, OrynXsltOutput *output) {
    initialize_once();
    memset(output, 0, sizeof(*output));
    if (count > 1024) return 0;
    if (!begin_budget()) return 0;
    resources = loaded; resource_count = loaded_count;
    xsltStylesheetPtr sheet = compile(style, style_length, style_url);
    xmlDocPtr input = NULL, result = NULL;
    xsltTransformContextPtr context = NULL;
    xsltSecurityPrefsPtr security = NULL;
    xmlChar *serialized = NULL;
    int length = 0, ok = 0;
    if (!sheet) goto done;
    input = parse_at(source, source_length, style_url);
    if (!input) goto done;
    security = xsltNewSecurityPrefs();
    if (!security) goto done;
    for (int option = XSLT_SECPREF_READ_FILE; option <= XSLT_SECPREF_WRITE_NETWORK; ++option)
        if (xsltSetSecurityPrefs(security, (xsltSecurityOption)option, xsltSecurityForbid) != 0) goto done;
    context = xsltNewTransformContext(sheet, input);
    if (!context || xsltSetCtxtSecurityPrefs(security, context) != 0) goto done;
    context->maxTemplateDepth = 256;
    context->maxTemplateVars = 16384;
    context->opLimit = 1000000;
    /* document() is exposed as an XSLT1 function but returns the empty node set. */
    /* libxslt already registered this function. libxml2 rejects duplicate
     * registrations; remove it before installing the restricted replacement. */
    if (xmlXPathRegisterFunc(context->xpathCtxt, BAD_CAST "document", NULL) != 0) goto done;
    if (xmlXPathRegisterFunc(context->xpathCtxt, BAD_CAST "document", empty_document) != 0) goto done;
    for (size_t i = 0; i < count; ++i) {
        const char *name = parameters[i].local;
        char *qualified = NULL;
        if (parameters[i].namespace_uri[0]) {
            char prefix[64]; size_t suffix = 0;
            xmlNodePtr root = xmlDocGetRootElement(sheet->doc);
            do {
                snprintf(prefix, sizeof(prefix), "_oryn_parameter_%zu_%zu", i, suffix++);
            } while (xmlSearchNs(sheet->doc, root, BAD_CAST prefix));
            if (!xmlNewNs(root, BAD_CAST parameters[i].namespace_uri, BAD_CAST prefix)) goto done;
            size_t size = strlen(prefix) + strlen(name) + 2;
            qualified = malloc(size);
            if (!qualified) goto done;
            snprintf(qualified, size, "%s:%s", prefix, name);
            name = qualified;
        }
        int status = xsltQuoteOneUserParam(context, BAD_CAST name, BAD_CAST parameters[i].value);
        free(qualified);
        if (status != 0) goto done;
    }
    /* Encoding describes an external byte stream; DOM strings are always UTF-8 here. */
    if (sheet->encoding) xmlFree(sheet->encoding);
    sheet->encoding = xmlStrdup(BAD_CAST "UTF-8");
    if (!sheet->encoding) goto done;
    result = xsltApplyStylesheetUser(sheet, input, NULL, NULL, NULL, context);
    if (!result || context->state != XSLT_STATE_OK) goto done;
    output->kind = sheet->method && xmlStrEqual(sheet->method, BAD_CAST "text") ? 2
                 : result->type == XML_HTML_DOCUMENT_NODE ? 1 : 0;
    if (output->kind == 2) {
        if (xsltSaveResultToString(&serialized, &length, result, sheet) < 0) goto done;
    } else {
        /* DOM results contain actual result-tree nodes, not XML declaration,
         * pretty-print whitespace, or serializer-added document newlines. */
        xmlBufferPtr buffer = xmlBufferCreate();
        if (!buffer) goto done;
        xmlOutputBufferPtr sink = xmlOutputBufferCreateBuffer(buffer, NULL);
        if (!sink) { xmlBufferFree(buffer); goto done; }
        for (xmlNodePtr node = result->children; node && !sink->error; node = node->next) {
            if (output->kind == 1) htmlNodeDumpFormatOutput(sink, result, node, "UTF-8", 0);
            else xmlNodeDumpOutput(sink, result, node, 0, 0, "UTF-8");
        }
        xmlOutputBufferFlush(sink);
        int failed = sink->error;
        length = xmlBufferLength(buffer);
        if (!failed && length >= 0 && length <= 16 * 1024 * 1024)
            serialized = length == 0 ? xmlStrdup(BAD_CAST "") : xmlStrndup(xmlBufferContent(buffer), length);
        xmlOutputBufferClose(sink); xmlBufferFree(buffer);
        if (failed || !serialized) goto done;
    }
    if (length < 0 || length > 16 * 1024 * 1024) goto done;
    output->bytes = malloc((size_t)length + 1);
    if (!output->bytes) goto done;
    if (length > 0) {
        if (!serialized) { free(output->bytes); output->bytes = NULL; goto done; }
        memcpy(output->bytes, serialized, (size_t)length);
    }
    output->bytes[length] = 0;
    output->length = (size_t)length;
    ok = 1;
done:
    if (serialized) xmlFree(serialized);
    if (result) xmlFreeDoc(result);
    if (context) xsltFreeTransformContext(context);
    if (security) xsltFreeSecurityPrefs(security);
    if (input) xmlFreeDoc(input);
    if (sheet) xsltFreeStylesheet(sheet);
    resources = NULL; resource_count = 0;
    if (!end_budget()) { free(output->bytes); memset(output, 0, sizeof(*output)); return 0; }
    return ok;
}
void oryn_xslt_free(unsigned char *bytes) { free(bytes); }
