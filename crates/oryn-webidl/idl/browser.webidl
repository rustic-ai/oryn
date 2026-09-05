[Exposed=(Window,Worker)]
interface EventTarget {
  constructor();
  undefined addEventListener(DOMString type, EventListener? callback, optional (boolean or AddEventListenerOptions) options);
  undefined removeEventListener(DOMString type, EventListener? callback, optional (boolean or EventListenerOptions) options);
  boolean dispatchEvent(Event event);
};

dictionary EventListenerOptions {
  boolean capture = false;
};

dictionary AddEventListenerOptions : EventListenerOptions {
  boolean passive;
  boolean once = false;
  AbortSignal? signal;
};

dictionary EventInit {
  boolean bubbles = false;
  boolean cancelable = false;
  boolean composed = false;
};

[Exposed=(Window,Worker)]
interface DOMException {
  constructor(optional DOMString message = "", optional DOMString name = "Error");
  readonly attribute DOMString name;
  readonly attribute DOMString message;
  readonly attribute unsigned short code;
  const unsigned short INDEX_SIZE_ERR = 1;
  const unsigned short DOMSTRING_SIZE_ERR = 2;
  const unsigned short HIERARCHY_REQUEST_ERR = 3;
  const unsigned short WRONG_DOCUMENT_ERR = 4;
  const unsigned short INVALID_CHARACTER_ERR = 5;
  const unsigned short NO_DATA_ALLOWED_ERR = 6;
  const unsigned short NO_MODIFICATION_ALLOWED_ERR = 7;
  const unsigned short NOT_FOUND_ERR = 8;
  const unsigned short NOT_SUPPORTED_ERR = 9;
  const unsigned short INUSE_ATTRIBUTE_ERR = 10;
  const unsigned short INVALID_STATE_ERR = 11;
  const unsigned short SYNTAX_ERR = 12;
  const unsigned short INVALID_MODIFICATION_ERR = 13;
  const unsigned short NAMESPACE_ERR = 14;
  const unsigned short INVALID_ACCESS_ERR = 15;
  const unsigned short VALIDATION_ERR = 16;
  const unsigned short TYPE_MISMATCH_ERR = 17;
  const unsigned short SECURITY_ERR = 18;
  const unsigned short NETWORK_ERR = 19;
  const unsigned short ABORT_ERR = 20;
  const unsigned short URL_MISMATCH_ERR = 21;
  const unsigned short QUOTA_EXCEEDED_ERR = 22;
  const unsigned short TIMEOUT_ERR = 23;
  const unsigned short INVALID_NODE_TYPE_ERR = 24;
  const unsigned short DATA_CLONE_ERR = 25;
};

dictionary QuotaExceededErrorOptions {
  double quota;
  double requested;
};

[Exposed=(Window,Worker)]
interface QuotaExceededError : DOMException {
  constructor(optional DOMString message = "", optional any options = null);
  readonly attribute double? quota;
  readonly attribute double? requested;
};

[Exposed=(Window,Worker)]
interface URL {
  constructor(DOMString url, optional DOMString base);
  attribute DOMString href;
  readonly attribute DOMString origin;
  readonly attribute DOMString protocol;
  readonly attribute DOMString username;
  readonly attribute DOMString password;
  readonly attribute DOMString host;
  readonly attribute DOMString hostname;
  readonly attribute DOMString port;
  readonly attribute DOMString pathname;
  readonly attribute DOMString search;
  readonly attribute DOMString hash;
  DOMString toJSON();
  DOMString toString();
};

[Exposed=(Window,Worker)]
interface Event {
  const unsigned short NONE = 0;
  const unsigned short CAPTURING_PHASE = 1;
  const unsigned short AT_TARGET = 2;
  const unsigned short BUBBLING_PHASE = 3;
  constructor(DOMString type, optional EventInit eventInitDict);
  readonly attribute DOMString type;
  readonly attribute EventTarget? target;
  readonly attribute EventTarget? srcElement;
  readonly attribute EventTarget? currentTarget;
  readonly attribute unsigned short eventPhase;
  readonly attribute boolean bubbles;
  readonly attribute boolean cancelable;
  [LegacyUnforgeable] readonly attribute boolean isTrusted;
  readonly attribute boolean defaultPrevented;
  attribute boolean cancelBubble;
  attribute boolean returnValue;
  readonly attribute double timeStamp;
  sequence<EventTarget> composedPath();
  undefined initEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false);
  undefined preventDefault();
  undefined stopPropagation();
  undefined stopImmediatePropagation();
};

[Exposed=(Window,Worker)]
interface CustomEvent : Event {
  constructor(DOMString type, optional CustomEventInit eventInitDict);
  readonly attribute any detail;
  undefined initCustomEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false, optional any detail = null);
};

dictionary CustomEventInit : EventInit {
  any detail = null;
};

dictionary MouseEventInit {
  boolean bubbles = false;
  boolean cancelable = false;
  any view;
};

[Exposed=Window]
interface MouseEvent : Event {
  constructor(DOMString type, optional MouseEventInit eventInitDict);
};

dictionary InputEventInit {
  boolean bubbles = true;
  boolean cancelable = false;
  DOMString? data = null;
  DOMString inputType = "";
  boolean isComposing = false;
  any dataTransfer;
};

[Exposed=Window]
interface InputEvent : Event {
  constructor(DOMString type, optional InputEventInit eventInitDict);
  readonly attribute DOMString? data;
  readonly attribute DOMString inputType;
  readonly attribute boolean isComposing;
  readonly attribute any dataTransfer;
  sequence<StaticRange> getTargetRanges();
};

[Exposed=(Window,Worker)]
interface MessagePort : EventTarget {
  attribute EventListener? onmessage;
  undefined postMessage(any message, optional sequence<object> transfer = []);
  undefined start();
  undefined close();
};

[Exposed=(Window,Worker)]
interface MessageChannel {
  constructor();
  readonly attribute MessagePort port1;
  readonly attribute MessagePort port2;
};

callback MutationCallback = undefined (sequence<MutationRecord> mutations, MutationObserver observer);

dictionary MutationObserverInit {
  boolean childList = false;
  boolean attributes = false;
  boolean characterData = false;
  boolean subtree = false;
  boolean attributeOldValue = false;
  boolean characterDataOldValue = false;
  sequence<DOMString> attributeFilter = [];
};

[Exposed=Window]
interface MutationRecord {
  readonly attribute DOMString type;
  readonly attribute Node target;
  readonly attribute sequence<Node> addedNodes;
  readonly attribute sequence<Node> removedNodes;
  readonly attribute Node? previousSibling;
  readonly attribute Node? nextSibling;
  readonly attribute DOMString? attributeName;
  readonly attribute DOMString? attributeNamespace;
  readonly attribute DOMString? oldValue;
};

[Exposed=Window]
interface MutationObserver {
  constructor(MutationCallback callback);
  undefined observe(Node target, MutationObserverInit options);
  undefined disconnect();
  sequence<MutationRecord> takeRecords();
};

enum SecurityPolicyViolationEventDisposition { "enforce", "report" };

dictionary SecurityPolicyViolationEventInit {
  DOMString documentURI = "";
  DOMString referrer = "";
  DOMString blockedURI = "";
  DOMString violatedDirective = "";
  DOMString effectiveDirective = "";
  DOMString originalPolicy = "";
  DOMString sourceFile = "";
  DOMString sample = "";
  SecurityPolicyViolationEventDisposition disposition = "enforce";
  unsigned short statusCode = 0;
  unsigned long lineNumber = 0;
  unsigned long columnNumber = 0;
};

[Exposed=Window]
interface SecurityPolicyViolationEvent : Event {
  constructor(DOMString type, optional SecurityPolicyViolationEventInit eventInitDict);
  readonly attribute DOMString documentURI;
  readonly attribute DOMString referrer;
  readonly attribute DOMString blockedURI;
  readonly attribute DOMString violatedDirective;
  readonly attribute DOMString effectiveDirective;
  readonly attribute DOMString originalPolicy;
  readonly attribute DOMString sourceFile;
  readonly attribute DOMString sample;
  readonly attribute SecurityPolicyViolationEventDisposition disposition;
  readonly attribute unsigned short statusCode;
  readonly attribute unsigned long lineNumber;
  readonly attribute unsigned long columnNumber;
};

[Exposed=Window]
interface Node : EventTarget {
  const unsigned short ELEMENT_NODE = 1;
  const unsigned short ATTRIBUTE_NODE = 2;
  const unsigned short TEXT_NODE = 3;
  const unsigned short CDATA_SECTION_NODE = 4;
  const unsigned short ENTITY_REFERENCE_NODE = 5;
  const unsigned short ENTITY_NODE = 6;
  const unsigned short PROCESSING_INSTRUCTION_NODE = 7;
  const unsigned short COMMENT_NODE = 8;
  const unsigned short DOCUMENT_NODE = 9;
  const unsigned short DOCUMENT_TYPE_NODE = 10;
  const unsigned short DOCUMENT_FRAGMENT_NODE = 11;
  const unsigned short NOTATION_NODE = 12;
  const unsigned short DOCUMENT_POSITION_DISCONNECTED = 1;
  const unsigned short DOCUMENT_POSITION_PRECEDING = 2;
  const unsigned short DOCUMENT_POSITION_FOLLOWING = 4;
  const unsigned short DOCUMENT_POSITION_CONTAINS = 8;
  const unsigned short DOCUMENT_POSITION_CONTAINED_BY = 16;
  const unsigned short DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
  readonly attribute DOMString nodeName;
  readonly attribute unsigned short nodeType;
  readonly attribute Document? ownerDocument;
  readonly attribute Node? parentNode;
  readonly attribute Element? parentElement;
  readonly attribute Node? firstChild;
  readonly attribute Node? lastChild;
  readonly attribute Node? previousSibling;
  readonly attribute Node? nextSibling;
  readonly attribute sequence<Node> childNodes;
  readonly attribute boolean isConnected;
  attribute DOMString? nodeValue;
  attribute DOMString? textContent;
  Node appendChild(Node child);
  Node insertBefore(Node child, Node? reference);
  Node removeChild(Node child);
  Node replaceChild(Node child, Node replaced);
  Node cloneNode(optional boolean deep = false);
  boolean contains(Node? other);
  boolean isEqualNode(Node? other);
  unsigned short compareDocumentPosition(Node other);
  boolean hasChildNodes();
  Node getRootNode();
  undefined append(DOMString node);
  undefined before(Node node);
  undefined after(Node node);
  undefined remove();
};

dictionary ShadowRootInit {
  DOMString mode = "open";
  boolean clonable = false;
};

[Exposed=Window]
interface Element : Node {
  readonly attribute DOMString tagName;
  readonly attribute DOMString localName;
  readonly attribute DOMString? namespaceURI;
  readonly attribute sequence<any> attributes;
  readonly attribute sequence<Element> children;
  readonly attribute unsigned long childElementCount;
  readonly attribute Element? firstElementChild;
  readonly attribute Element? lastElementChild;
  readonly attribute Element? previousElementSibling;
  readonly attribute Element? nextElementSibling;
  attribute DOMString id;
  attribute DOMString className;
  readonly attribute DOMTokenList classList;
  attribute DOMString innerHTML;
  attribute DOMString outerHTML;
  readonly attribute ShadowRoot? shadowRoot;
  ShadowRoot attachShadow(ShadowRootInit init);
  DOMString? getAttribute(DOMString name);
  DOMString? getAttributeNS(DOMString? namespace, DOMString localName);
  Attr? getAttributeNode(DOMString name);
  sequence<DOMString> getAttributeNames();
  undefined setAttribute(DOMString name, DOMString value);
  undefined setAttributeNS(DOMString? namespace, DOMString qualifiedName, DOMString value);
  undefined removeAttribute(DOMString name);
  undefined removeAttributeNS(DOMString? namespace, DOMString localName);
  boolean hasAttribute(DOMString name);
  boolean hasAttributeNS(DOMString? namespace, DOMString localName);
  boolean hasAttributes();
  boolean toggleAttribute(DOMString name, optional boolean force);
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
  sequence<Element> getElementsByTagName(DOMString qualifiedName);
  sequence<Element> getElementsByClassName(DOMString classNames);
  boolean matches(DOMString selectors);
  Element? closest(DOMString selectors);
  Element? insertAdjacentElement(DOMString where, Element element);
  undefined insertAdjacentText(DOMString where, DOMString data);
  undefined insertAdjacentHTML(DOMString where, DOMString html);
  DOMRect getBoundingClientRect();
  sequence<DOMRect> getClientRects();
  attribute double scrollLeft;
  attribute double scrollTop;
  undefined scrollTo(optional double x = 0, optional double y = 0);
  undefined scrollIntoView(optional boolean alignToTop = true);
  undefined remove();
};

[Exposed=Window]
interface Attr : Node {
  readonly attribute DOMString name;
  readonly attribute DOMString localName;
  readonly attribute DOMString? namespaceURI;
  readonly attribute DOMString? prefix;
  readonly attribute boolean specified;
  attribute DOMString value;
  readonly attribute Element? ownerElement;
};

[Exposed=Window]
interface DocumentType : Node {
  readonly attribute DOMString name;
  readonly attribute DOMString publicId;
  readonly attribute DOMString systemId;
};

[Exposed=Window]
interface SVGElement : Element {
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleDeclaration style;
};

[Exposed=Window]
interface SVGAElement : SVGElement {
};

[Exposed=Window]
interface HTMLBodyElement : HTMLElement {
  attribute USVString background;
};

[Exposed=Window]
interface HTMLMetaElement : HTMLElement {
  [Reflect] attribute DOMString name;
  [Reflect] attribute DOMString content;
  [Reflect] attribute DOMString httpEquiv;
};

[Exposed=Window]
interface DOMRect {
  constructor(optional double x = 0, optional double y = 0, optional double width = 0, optional double height = 0);
  readonly attribute double x;
  readonly attribute double y;
  readonly attribute double width;
  readonly attribute double height;
  readonly attribute double top;
  readonly attribute double right;
  readonly attribute double bottom;
  readonly attribute double left;
};

[Exposed=(Window,Worker)]
interface DOMRectReadOnly {
  constructor(optional double x = 0, optional double y = 0, optional double width = 0, optional double height = 0);
};

[Exposed=(Window,Worker)]
interface DOMPoint {
  constructor(optional double x = 0, optional double y = 0, optional double z = 0, optional double w = 1);
};

[Exposed=(Window,Worker)]
interface DOMPointReadOnly {
  constructor(optional double x = 0, optional double y = 0, optional double z = 0, optional double w = 1);
};

[Exposed=(Window,Worker)]
interface DOMMatrix {
  constructor(optional any init = null);
};

[Exposed=(Window,Worker)]
interface DOMMatrixReadOnly {
  constructor(optional any init = null);
};

[Exposed=(Window,Worker)]
interface ImageData {
  constructor(unsigned long sw, unsigned long sh);
  readonly attribute unsigned long width;
  readonly attribute unsigned long height;
  readonly attribute Uint8ClampedArray data;
};

[Exposed=Window]
interface DOMTokenList {
  attribute DOMString value;
  undefined add(DOMString token);
  undefined remove(DOMString token);
  boolean toggle(DOMString token, optional boolean force);
  boolean contains(DOMString token);
};

[Exposed=Window]
interface HTMLElement : Element {
  attribute DOMString innerText;
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleDeclaration style;
  readonly attribute any dataset;
  attribute EventListener? onclick;
  attribute EventListener? oninput;
  attribute EventListener? onchange;
  attribute EventListener? onsubmit;
  attribute EventListener? onfocus;
  attribute EventListener? onblur;
  attribute EventListener? onkeydown;
  attribute EventListener? onkeyup;
  attribute EventListener? onload;
  attribute EventListener? onerror;
  attribute DOMString contentEditable;
  readonly attribute boolean isContentEditable;
  undefined click();
  undefined focus();
  undefined blur();
};

[Exposed=Window]
interface HTMLTemplateElement : HTMLElement {
  readonly attribute DocumentFragment content;
};

[Exposed=Window]
interface HTMLMediaElement : HTMLElement {
};

[Exposed=Window]
interface HTMLCanvasElement : HTMLElement {
  any getContext(DOMString contextId);
};

[Exposed=Window]
interface HTMLImageElement : HTMLElement {
  attribute DOMString src;
  attribute DOMString crossOrigin;
  attribute DOMString referrerPolicy;
  readonly attribute unsigned long naturalWidth;
  readonly attribute unsigned long naturalHeight;
  readonly attribute unsigned long clientWidth;
  readonly attribute unsigned long clientHeight;
};

[Exposed=Window]
interface HTMLInputElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString defaultValue;
  attribute DOMString type;
  attribute DOMString name;
  attribute boolean checked;
  attribute boolean defaultChecked;
  attribute boolean disabled;
  attribute boolean readOnly;
  attribute boolean required;
  attribute unsigned long? selectionStart;
  attribute unsigned long? selectionEnd;
  attribute DOMString? selectionDirection;
  undefined select();
  undefined setSelectionRange(unsigned long start, unsigned long end, optional DOMString direction = "none");
};

[Exposed=Window]
interface HTMLTextAreaElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString defaultValue;
  attribute DOMString name;
  attribute boolean disabled;
  attribute boolean readOnly;
  attribute boolean required;
  attribute unsigned long? selectionStart;
  attribute unsigned long? selectionEnd;
  attribute DOMString? selectionDirection;
  undefined select();
  undefined setSelectionRange(unsigned long start, unsigned long end, optional DOMString direction = "none");
};

[Exposed=Window]
interface HTMLSelectElement : HTMLElement {
  readonly attribute sequence<HTMLOptionElement> options;
  attribute DOMString value;
  attribute long selectedIndex;
  attribute DOMString name;
  attribute boolean disabled;
  attribute boolean multiple;
  attribute boolean required;
};

[Exposed=Window]
interface HTMLOptionElement : HTMLElement {
  attribute DOMString value;
  attribute boolean selected;
  attribute boolean defaultSelected;
  attribute boolean disabled;
};

[Exposed=Window]
interface HTMLButtonElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString type;
  attribute DOMString name;
  attribute boolean disabled;
};

[Exposed=Window]
interface HTMLFormElement : HTMLElement {
  attribute DOMString action;
  attribute DOMString method;
  boolean checkValidity();
  boolean reportValidity();
  undefined requestSubmit(optional HTMLElement? submitter = null);
  undefined submit();
  undefined reset();
};

[Exposed=Window]
interface HTMLAnchorElement : HTMLElement {
  attribute DOMString href;
  attribute DOMString target;
  attribute DOMString referrerPolicy;
};

[Exposed=Window]
interface HTMLAreaElement : HTMLElement {
  attribute DOMString href;
  attribute DOMString target;
  attribute DOMString referrerPolicy;
};

[Exposed=Window]
interface HTMLLinkElement : HTMLElement {
  attribute boolean disabled;
  attribute DOMString href;
  attribute DOMString rel;
  attribute DOMString media;
  attribute DOMString type;
  attribute DOMString referrerPolicy;
  readonly attribute CSSStyleSheet? sheet;
};

[Exposed=Window]
interface HTMLStyleElement : HTMLElement {
  attribute boolean disabled;
  attribute DOMString media;
  attribute DOMString type;
  readonly attribute CSSStyleSheet? sheet;
};

[Exposed=Window]
interface HTMLIFrameElement : HTMLElement {
  attribute DOMString name;
  attribute DOMString src;
  attribute DOMString srcdoc;
  attribute DOMString sandbox;
  attribute DOMString referrerPolicy;
  readonly attribute Document? contentDocument;
  readonly attribute any contentWindow;
};

[Exposed=Window]
interface HTMLScriptElement : HTMLElement {
  attribute DOMString src;
  attribute DOMString type;
  attribute DOMString referrerPolicy;
};

[Exposed=Window]
interface Document : Node {
  constructor();
  readonly attribute DOMImplementation implementation;
  readonly attribute DOMString URL;
  readonly attribute DOMString baseURI;
  readonly attribute DOMString referrer;
  readonly attribute DOMString characterSet;
  readonly attribute DOMString charset;
  readonly attribute DOMString inputEncoding;
  readonly attribute DOMString contentType;
  readonly attribute any location;
  attribute DOMString cookie;
  readonly attribute any defaultView;
  readonly attribute HTMLElement? documentElement;
  readonly attribute HTMLElement? head;
  readonly attribute HTMLElement? body;
  readonly attribute Element? scrollingElement;
  readonly attribute Node? doctype;
  readonly attribute HTMLElement? activeElement;
  readonly attribute HTMLElement? currentScript;
  readonly attribute sequence<CSSStyleSheet> styleSheets;
  attribute DOMString designMode;
  Element createElement(DOMString localName);
  Element createElementNS(DOMString? namespace, DOMString qualifiedName);
  Text createTextNode(DOMString data);
  Comment createComment(DOMString data);
  Text createCDATASection(DOMString data);
  Comment createProcessingInstruction(DOMString target, DOMString data);
  DocumentFragment createDocumentFragment();
  Event createEvent(DOMString interfaceName);
  Document open();
  undefined write(DOMString text);
  undefined writeln(DOMString text);
  undefined close();
  Element? getElementById(DOMString id);
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
  sequence<Element> getElementsByTagName(DOMString qualifiedName);
  sequence<Element> getElementsByClassName(DOMString classNames);
  sequence<Element> getElementsByName(DOMString elementName);
  Element? elementFromPoint(double x, double y);
  sequence<Element> elementsFromPoint(double x, double y);
  Range createRange();
  NodeIterator createNodeIterator(Node root, optional unsigned long whatToShow = 4294967295, optional NodeFilter? filter = null);
  TreeWalker createTreeWalker(Node root, optional unsigned long whatToShow = 4294967295, optional NodeFilter? filter = null);
  Selection? getSelection();
  boolean execCommand(DOMString commandId, optional boolean showUI = false, optional DOMString value = "");
  boolean queryCommandEnabled(DOMString commandId);
  boolean queryCommandIndeterm(DOMString commandId);
  boolean queryCommandState(DOMString commandId);
  boolean queryCommandSupported(DOMString commandId);
  DOMString queryCommandValue(DOMString commandId);
};

[Exposed=Window]
interface DOMParser {
  constructor();
  Document parseFromString(DOMString input, DOMString type);
};

[Exposed=Window]
interface DOMImplementation {
  Document createDocument(DOMString? namespace, DOMString? qualifiedName, optional Node? doctype = null);
  Document createHTMLDocument(optional DOMString title = "");
  Node createDocumentType(DOMString qualifiedName, optional DOMString publicId = "", optional DOMString systemId = "");
};

[Exposed=Window]
interface AbstractRange {
  readonly attribute Node startContainer;
  readonly attribute unsigned long startOffset;
  readonly attribute Node endContainer;
  readonly attribute unsigned long endOffset;
  readonly attribute boolean collapsed;
};

dictionary StaticRangeInit {
  required Node startContainer;
  required unsigned long startOffset;
  required Node endContainer;
  required unsigned long endOffset;
};

[Exposed=Window]
interface StaticRange : AbstractRange {
  constructor(StaticRangeInit init);
};

[Exposed=Window]
interface Range : AbstractRange {
  constructor();
  const unsigned short START_TO_START = 0;
  const unsigned short START_TO_END = 1;
  const unsigned short END_TO_END = 2;
  const unsigned short END_TO_START = 3;
  readonly attribute Node commonAncestorContainer;
  undefined setStart(Node node, unsigned long offset);
  undefined setEnd(Node node, unsigned long offset);
  undefined setStartBefore(Node node);
  undefined setStartAfter(Node node);
  undefined setEndBefore(Node node);
  undefined setEndAfter(Node node);
  undefined collapse(optional boolean toStart = false);
  undefined selectNode(Node node);
  undefined selectNodeContents(Node node);
  short compareBoundaryPoints(unsigned short how, Range sourceRange);
  undefined deleteContents();
  DocumentFragment extractContents();
  DocumentFragment cloneContents();
  undefined insertNode(Node node);
  undefined surroundContents(Node newParent);
  Range cloneRange();
  undefined detach();
  boolean isPointInRange(Node node, unsigned long offset);
  short comparePoint(Node node, unsigned long offset);
  boolean intersectsNode(Node node);
  DOMString toString();
  DocumentFragment createContextualFragment(DOMString fragment);
};

[Exposed=Window]
interface Selection {
  readonly attribute Node? anchorNode;
  readonly attribute unsigned long anchorOffset;
  readonly attribute Node? focusNode;
  readonly attribute unsigned long focusOffset;
  readonly attribute boolean isCollapsed;
  readonly attribute unsigned long rangeCount;
  readonly attribute DOMString type;
  readonly attribute DOMString direction;
  Range getRangeAt(unsigned long index);
  undefined addRange(Range range);
  undefined removeRange(Range range);
  undefined removeAllRanges();
  undefined empty();
  undefined collapse(Node? node, optional unsigned long offset = 0);
  undefined setPosition(Node? node, optional unsigned long offset = 0);
  undefined extend(Node node, optional unsigned long offset = 0);
  undefined setBaseAndExtent(Node anchorNode, unsigned long anchorOffset, Node focusNode, unsigned long focusOffset);
  undefined modify(optional DOMString alter = "move", optional DOMString direction = "forward", optional DOMString granularity = "character");
  undefined collapseToStart();
  undefined collapseToEnd();
  undefined selectAllChildren(Node node);
  undefined deleteFromDocument();
  boolean containsNode(Node node, optional boolean allowPartialContainment = false);
  DOMString toString();
};

[Exposed=Window]
interface NodeFilter {
  const unsigned short FILTER_ACCEPT = 1;
  const unsigned short FILTER_REJECT = 2;
  const unsigned short FILTER_SKIP = 3;
  const unsigned long SHOW_ALL = 4294967295;
  const unsigned long SHOW_ELEMENT = 1;
  const unsigned long SHOW_ATTRIBUTE = 2;
  const unsigned long SHOW_TEXT = 4;
  const unsigned long SHOW_CDATA_SECTION = 8;
  const unsigned long SHOW_ENTITY_REFERENCE = 16;
  const unsigned long SHOW_ENTITY = 32;
  const unsigned long SHOW_PROCESSING_INSTRUCTION = 64;
  const unsigned long SHOW_COMMENT = 128;
  const unsigned long SHOW_DOCUMENT = 256;
  const unsigned long SHOW_DOCUMENT_TYPE = 512;
  const unsigned long SHOW_DOCUMENT_FRAGMENT = 1024;
  const unsigned long SHOW_NOTATION = 2048;
  unsigned short acceptNode(Node node);
};

[Exposed=Window]
interface NodeIterator {
  readonly attribute Node root;
  readonly attribute Node referenceNode;
  readonly attribute boolean pointerBeforeReferenceNode;
  readonly attribute unsigned long whatToShow;
  Node? nextNode();
  Node? previousNode();
  undefined detach();
};

[Exposed=Window]
interface TreeWalker {
  readonly attribute Node root;
  readonly attribute unsigned long whatToShow;
  attribute Node currentNode;
  Node? parentNode();
  Node? firstChild();
  Node? lastChild();
  Node? previousSibling();
  Node? nextSibling();
  Node? previousNode();
  Node? nextNode();
};

[Exposed=Window]
interface History {
  readonly attribute unsigned long length;
  readonly attribute any state;
  attribute DOMString scrollRestoration;
  undefined go(optional long delta = 0);
  undefined back();
  undefined forward();
  undefined pushState(any data, DOMString unused, optional DOMString? url = null);
  undefined replaceState(any data, DOMString unused, optional DOMString? url = null);
};

[Exposed=(Window,Worker)]
interface Storage {
  readonly attribute unsigned long length;
  DOMString? key(unsigned long index);
  DOMString? getItem(DOMString key);
  undefined setItem(DOMString key, DOMString value);
  undefined removeItem(DOMString key);
  undefined clear();
};

[Exposed=(Window,Worker)]
interface IDBFactory {
  IDBOpenDBRequest open(DOMString name, optional unsigned long long version);
  IDBOpenDBRequest deleteDatabase(DOMString name);
  short cmp(any first, any second);
  any databases();
};

[Exposed=(Window,Worker)]
interface IDBVersionChangeEvent : Event {
  constructor(DOMString type, optional any eventInitDict);
  readonly attribute unsigned long long oldVersion;
  readonly attribute any newVersion;
};

[Exposed=(Window,Worker)]
interface IDBRequest : EventTarget {
  readonly attribute any result;
  readonly attribute any error;
  readonly attribute any source;
  readonly attribute IDBTransaction? transaction;
  readonly attribute DOMString readyState;
  attribute EventListener? onsuccess;
  attribute EventListener? onerror;
};

[Exposed=(Window,Worker)]
interface IDBOpenDBRequest : IDBRequest {
  attribute EventListener? onblocked;
  attribute EventListener? onupgradeneeded;
};

[Exposed=(Window,Worker)]
interface IDBDatabase : EventTarget {
  readonly attribute DOMString name;
  readonly attribute unsigned long long version;
  readonly attribute sequence<DOMString> objectStoreNames;
  attribute EventListener? onabort;
  attribute EventListener? onclose;
  attribute EventListener? onerror;
  attribute EventListener? onversionchange;
  IDBObjectStore createObjectStore(DOMString name, optional any options);
  undefined deleteObjectStore(DOMString name);
  IDBTransaction transaction((DOMString or sequence<DOMString>) storeNames, optional DOMString mode = "readonly");
  undefined close();
};

[Exposed=(Window,Worker)]
interface IDBTransaction : EventTarget {
  readonly attribute IDBDatabase db;
  readonly attribute DOMString mode;
  readonly attribute DOMString durability;
  readonly attribute any error;
  readonly attribute sequence<DOMString> objectStoreNames;
  attribute EventListener? onabort;
  attribute EventListener? oncomplete;
  attribute EventListener? onerror;
  IDBObjectStore objectStore(DOMString name);
  undefined abort();
  undefined commit();
};

[Exposed=(Window,Worker)]
interface IDBKeyRange {
  readonly attribute any lower;
  readonly attribute any upper;
  readonly attribute boolean lowerOpen;
  readonly attribute boolean upperOpen;
  boolean includes(any key);
};

[Exposed=(Window,Worker)]
interface IDBCursor {
  readonly attribute any source;
  readonly attribute DOMString direction;
  readonly attribute any key;
  readonly attribute any primaryKey;
  readonly attribute IDBRequest request;
  undefined advance(unsigned long count);
  undefined continue(optional any key);
  undefined continuePrimaryKey(any key, any primaryKey);
  undefined delete();
  undefined update(any value);
};

[Exposed=(Window,Worker)]
interface IDBCursorWithValue : IDBCursor {
  readonly attribute any value;
};

[Exposed=(Window,Worker)]
interface IDBIndex {
  attribute DOMString name;
  readonly attribute IDBObjectStore objectStore;
  readonly attribute any keyPath;
  readonly attribute boolean multiEntry;
  readonly attribute boolean unique;
  any get(any query);
  any getKey(any query);
  any getAll(optional any query, optional unsigned long count);
  any getAllKeys(optional any query, optional unsigned long count);
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
};

[Exposed=(Window,Worker)]
interface IDBObjectStore {
  readonly attribute DOMString name;
  readonly attribute any keyPath;
  readonly attribute sequence<DOMString> indexNames;
  readonly attribute IDBTransaction transaction;
  readonly attribute boolean autoIncrement;
  any add(any value, optional any key);
  any put(any value, optional any key);
  any get(any query);
  any getKey(any query);
  any getAll(optional any query, optional unsigned long count);
  any getAllKeys(optional any query, optional unsigned long count);
  any delete(any query);
  any clear();
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
  IDBIndex createIndex(DOMString name, (DOMString or sequence<DOMString>) keyPath, optional any options);
  undefined deleteIndex(DOMString name);
  IDBIndex index(DOMString name);
};

[Exposed=Window]
interface CSSStyleSheet {
  constructor();
  readonly attribute sequence<any> cssRules;
  unsigned long insertRule(DOMString rule, optional unsigned long index = 0);
  undefined deleteRule(unsigned long index);
  undefined replaceSync(DOMString text);
  Promise<CSSStyleSheet> replace(DOMString text);
};

[Exposed=Window]
interface CSSStyleDeclaration {
  attribute DOMString cssText;
  readonly attribute unsigned long length;
  attribute DOMString display;
  attribute DOMString visibility;
  attribute DOMString width;
  attribute DOMString height;
  attribute DOMString position;
  attribute DOMString top;
  attribute DOMString left;
  attribute DOMString right;
  attribute DOMString bottom;
  attribute DOMString overflow;
  attribute DOMString opacity;
  attribute DOMString color;
  attribute DOMString border;
  attribute DOMString fontSize;
  attribute DOMString fontStyle;
  attribute DOMString fontWeight;
  attribute DOMString margin;
  attribute DOMString marginLeft;
  attribute DOMString padding;
  attribute DOMString textAlign;
  attribute DOMString textDecoration;
  attribute DOMString verticalAlign;
  attribute DOMString backgroundClip;
  attribute DOMString backgroundColor;
  attribute DOMString borderCollapse;
  attribute DOMString boxSizing;
  attribute DOMString cssFloat;
  attribute DOMString filter;
  attribute DOMString marginRight;
  attribute DOMString maxHeight;
  attribute DOMString minHeight;
  attribute DOMString whiteSpace;
  attribute DOMString MozBoxSizing;
  attribute DOMString WebkitBoxSizing;
  attribute DOMString zoom;
  DOMString getPropertyValue(DOMString property);
  DOMString getPropertyPriority(DOMString property);
  undefined setProperty(DOMString property, DOMString value, optional DOMString priority = "");
  DOMString removeProperty(DOMString property);
};

[Exposed=Window]
interface DocumentFragment : Node {
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
};

[Exposed=Window]
interface ShadowRoot : DocumentFragment {
  attribute DOMString innerHTML;
  readonly attribute DOMString mode;
  readonly attribute boolean clonable;
  readonly attribute Element host;
  Element? getElementById(DOMString id);
};

[Exposed=Window]
interface CharacterData : Node {
  attribute DOMString data;
  readonly attribute unsigned long length;
  DOMString substringData(unsigned long offset, unsigned long count);
  undefined appendData(DOMString data);
  undefined insertData(unsigned long offset, DOMString data);
  undefined deleteData(unsigned long offset, unsigned long count);
  undefined replaceData(unsigned long offset, unsigned long count, DOMString data);
};

[Exposed=Window]
interface Text : CharacterData {
  Text splitText(unsigned long offset);
};

[Exposed=Window]
interface CDATASection : Text {};

[Exposed=Window]
interface ProcessingInstruction : CharacterData {
  readonly attribute DOMString target;
};

[Exposed=Window]
interface Comment : CharacterData {
};

[Exposed=(Window,Worker)]
interface Blob {
  constructor(optional sequence<any> blobParts = [], optional any options = null);
  readonly attribute unsigned long long size;
  readonly attribute DOMString type;
  Blob slice(optional long long start = 0, optional long long end, optional DOMString contentType = "");
  Promise<ArrayBuffer> arrayBuffer();
  Promise<DOMString> text();
  ReadableStream stream();
};

[Exposed=(Window,Worker)]
interface File : Blob {
  constructor(sequence<any> fileBits, DOMString fileName, optional any options = null);
  readonly attribute DOMString name;
  readonly attribute long long lastModified;
  readonly attribute DOMString webkitRelativePath;
};

[Exposed=(Window,Worker)]
interface FileReader : EventTarget {
  constructor();
  readonly attribute any result;
  readonly attribute DOMException? error;
  readonly attribute unsigned short readyState;
  attribute EventListener? onload;
  attribute EventListener? onerror;
  attribute EventListener? onabort;
  attribute EventListener? onloadend;
  undefined readAsArrayBuffer(Blob blob);
  undefined readAsText(Blob blob, optional DOMString encoding);
  undefined readAsDataURL(Blob blob);
  undefined abort();
};

[Exposed=(Window,Worker)]
interface FormData {
  constructor(optional any form = null);
  undefined append(DOMString name, any value, optional DOMString filename);
  undefined delete(DOMString name);
  any get(DOMString name);
  sequence<any> getAll(DOMString name);
  boolean has(DOMString name);
  undefined set(DOMString name, any value, optional DOMString filename);
  iterable<DOMString, any>;
};

[Exposed=(Window,Worker)]
interface Headers {
  constructor(optional any init = null);
  undefined append(DOMString name, DOMString value);
  undefined delete(DOMString name);
  DOMString? get(DOMString name);
  boolean has(DOMString name);
  undefined set(DOMString name, DOMString value);
  iterable<DOMString, DOMString>;
};

[Exposed=(Window,Worker)]
interface AbortSignal : EventTarget {
  static AbortSignal abort(optional any reason);
  static AbortSignal timeout(unsigned long milliseconds);
  static AbortSignal any(sequence<AbortSignal> signals);
  readonly attribute boolean aborted;
  readonly attribute any reason;
  attribute EventListener? onabort;
  undefined throwIfAborted();
};

[Exposed=(Window,Worker)]
interface AbortController {
  constructor();
  readonly attribute AbortSignal signal;
  undefined abort(optional any reason = null);
};

[Exposed=(Window,Worker)]
interface ReadableStream {
  constructor(optional any underlyingSource = null, optional any strategy = null);
  readonly attribute boolean locked;
  ReadableStreamDefaultReader getReader();
  Promise<undefined> cancel(optional any reason = null);
  Promise<undefined> pipeTo(WritableStream destination);
  ReadableStream pipeThrough(TransformStream transform);
};

[Exposed=(Window,Worker)]
interface ReadableStreamDefaultReader {
  readonly attribute Promise<undefined> closed;
  Promise<any> read();
  Promise<undefined> cancel(optional any reason = null);
  undefined releaseLock();
};

[Exposed=(Window,Worker)]
interface ReadableStreamDefaultController {
  readonly attribute long desiredSize;
  undefined close();
  undefined enqueue(optional any chunk = null);
  undefined error(optional any error = null);
};

[Exposed=(Window,Worker)]
interface WritableStream {
  constructor(optional any underlyingSink = null, optional any strategy = null);
  readonly attribute boolean locked;
  WritableStreamDefaultWriter getWriter();
  Promise<undefined> abort(optional any reason = null);
  Promise<undefined> close();
};

[Exposed=(Window,Worker)]
interface WritableStreamDefaultWriter {
  readonly attribute Promise<undefined> closed;
  readonly attribute long desiredSize;
  readonly attribute Promise<undefined> ready;
  Promise<undefined> abort(optional any reason = null);
  Promise<undefined> close();
  Promise<undefined> write(optional any chunk = null);
  undefined releaseLock();
};

[Exposed=(Window,Worker)]
interface WritableStreamDefaultController {
  readonly attribute AbortSignal signal;
  undefined error(optional any error = null);
};

[Exposed=(Window,Worker)]
interface TransformStream {
  constructor(optional any transformer = null, optional any writableStrategy = null, optional any readableStrategy = null);
  readonly attribute ReadableStream readable;
  readonly attribute WritableStream writable;
};

[Exposed=(Window,Worker)]
interface Request {
  constructor(any input, optional any init = null);
  readonly attribute DOMString url;
  readonly attribute DOMString method;
  readonly attribute Headers headers;
  readonly attribute AbortSignal signal;
  readonly attribute DOMString credentials;
  readonly attribute DOMString referrer;
  readonly attribute DOMString referrerPolicy;
  readonly attribute any body;
  readonly attribute boolean bodyUsed;
  Request clone();
  Promise<ArrayBuffer> arrayBuffer();
  Promise<DOMString> text();
};

[Exposed=(Window,Worker)]
interface Response {
  constructor(optional any body = null, optional any init = null);
  readonly attribute unsigned short status;
  readonly attribute boolean ok;
  readonly attribute DOMString url;
  readonly attribute Headers headers;
  readonly attribute ReadableStream? body;
  readonly attribute boolean bodyUsed;
  Response clone();
  Promise<DOMString> text();
  Promise<any> json();
  Promise<ArrayBuffer> arrayBuffer();
  Promise<Blob> blob();
};

[Exposed=(Window,Worker)]
interface XMLHttpRequest : EventTarget {
  constructor();
  const unsigned short UNSENT = 0;
  const unsigned short OPENED = 1;
  const unsigned short HEADERS_RECEIVED = 2;
  const unsigned short LOADING = 3;
  const unsigned short DONE = 4;
  readonly attribute unsigned short readyState;
  attribute unsigned long timeout;
  attribute boolean withCredentials;
  attribute DOMString responseType;
  readonly attribute unsigned short status;
  readonly attribute DOMString responseURL;
  readonly attribute DOMString responseText;
  readonly attribute any response;
  attribute EventListener? onreadystatechange;
  attribute EventListener? onloadstart;
  attribute EventListener? onprogress;
  attribute EventListener? onabort;
  attribute EventListener? onerror;
  attribute EventListener? ontimeout;
  attribute EventListener? onload;
  attribute EventListener? onloadend;
  undefined open(DOMString method, DOMString url, optional boolean async = true);
  undefined setRequestHeader(DOMString name, DOMString value);
  DOMString? getResponseHeader(DOMString name);
  DOMString getAllResponseHeaders();
  undefined send(optional any body = null);
  undefined abort();
};

[Exposed=(Window,Worker)]
interface WebSocket : EventTarget {
  constructor(DOMString url, optional any protocols = null);
  const unsigned short CONNECTING = 0;
  const unsigned short OPEN = 1;
  const unsigned short CLOSING = 2;
  const unsigned short CLOSED = 3;
  readonly attribute DOMString url;
  readonly attribute unsigned short readyState;
  readonly attribute DOMString protocol;
  attribute DOMString binaryType;
  attribute EventListener? onopen;
  attribute EventListener? onmessage;
  attribute EventListener? onerror;
  attribute EventListener? onclose;
  undefined send(any data);
  undefined close(optional unsigned short code = 1000, optional DOMString reason = "");
};

dictionary WorkerOptions {
  DOMString type = "classic";
  DOMString name = "";
  DOMString credentials = "same-origin";
};

[Exposed=Window]
interface Worker : EventTarget {
  constructor(DOMString scriptURL, optional WorkerOptions options);
  undefined terminate();
  undefined postMessage(any message);
};

[Exposed=Window]
interface TrustedHTML {
  DOMString toString();
};

[Exposed=Window]
interface TrustedScript {
  DOMString toString();
};

[Exposed=Window]
interface TrustedScriptURL {
  DOMString toString();
};

dictionary TrustedTypePolicyOptions {
  TrustedTypePolicyCallback? createHTML = null;
  TrustedTypePolicyCallback? createScript = null;
  TrustedTypePolicyCallback? createScriptURL = null;
};

callback TrustedTypePolicyCallback = DOMString (DOMString input);

[Exposed=Window]
interface TrustedTypePolicy {
  readonly attribute DOMString name;
  TrustedHTML createHTML(DOMString input);
  TrustedScript createScript(DOMString input);
  TrustedScriptURL createScriptURL(DOMString input);
};

[Exposed=Window]
interface TrustedTypePolicyFactory {
  TrustedTypePolicy createPolicy(DOMString policyName, optional TrustedTypePolicyOptions? policyOptions = null);
  boolean isHTML(any value);
  boolean isScript(any value);
  boolean isScriptURL(any value);
  DOMString? getAttributeType(DOMString tagName, DOMString attribute, optional DOMString? elementNamespace = null, optional DOMString? attributeNamespace = null);
  DOMString? getPropertyType(DOMString tagName, DOMString property, optional DOMString? elementNamespace = null);
};

dictionary RequestInit {
  DOMString method = "GET";
  DOMString? body = null;
  DOMString referrer = "client";
  DOMString referrerPolicy = "";
};

callback EventListener = undefined (Event event);
