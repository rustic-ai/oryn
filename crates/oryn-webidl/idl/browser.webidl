[Exposed=Window]
interface EventTarget {
  undefined addEventListener(DOMString type, EventListener? callback);
  undefined removeEventListener(DOMString type, EventListener? callback);
  boolean dispatchEvent(Event event);
};

[Exposed=Window]
interface Event {
  readonly attribute DOMString type;
  readonly attribute EventTarget? target;
  readonly attribute EventTarget? currentTarget;
  readonly attribute boolean bubbles;
  readonly attribute boolean cancelable;
  readonly attribute boolean defaultPrevented;
  undefined initEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false);
  undefined preventDefault();
  undefined stopPropagation();
  undefined stopImmediatePropagation();
};

[Exposed=Window]
interface CustomEvent : Event {
};

callback MutationCallback = undefined (sequence<MutationRecord> mutations, MutationObserver observer);

dictionary MutationObserverInit {
  boolean childList = false;
  boolean attributes = false;
  boolean characterData = false;
  boolean subtree = false;
  boolean attributeOldValue = false;
  boolean characterDataOldValue = false;
};

[Exposed=Window]
interface MutationRecord {
  readonly attribute DOMString type;
  readonly attribute Node target;
  readonly attribute DOMString? attributeName;
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
  boolean hasChildNodes();
  Node getRootNode();
  undefined before(Node node);
  undefined after(Node node);
  undefined remove();
};

dictionary ShadowRootInit {
  DOMString mode = "open";
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
  sequence<DOMString> getAttributeNames();
  undefined setAttribute(DOMString name, DOMString value);
  undefined removeAttribute(DOMString name);
  boolean hasAttribute(DOMString name);
  boolean hasAttributes();
  boolean toggleAttribute(DOMString name, optional boolean force);
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
  sequence<Element> getElementsByTagName(DOMString qualifiedName);
  sequence<Element> getElementsByClassName(DOMString classNames);
  boolean matches(DOMString selectors);
  Element? closest(DOMString selectors);
  DOMRect getBoundingClientRect();
  sequence<DOMRect> getClientRects();
  undefined scrollIntoView(optional boolean alignToTop = true);
  undefined remove();
};

[Exposed=Window]
interface SVGElement : Element {
};

[Exposed=Window]
interface SVGAElement : SVGElement {
};

[Exposed=Window]
interface DOMRect {
  readonly attribute double x;
  readonly attribute double y;
  readonly attribute double width;
  readonly attribute double height;
  readonly attribute double top;
  readonly attribute double right;
  readonly attribute double bottom;
  readonly attribute double left;
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
  readonly attribute CSSStyleDeclaration style;
  readonly attribute any dataset;
  attribute EventListener? onclick;
  attribute EventListener? oninput;
  attribute EventListener? onchange;
  attribute EventListener? onsubmit;
  attribute EventListener? onfocus;
  attribute EventListener? onblur;
  attribute EventListener? onkeydown;
  attribute EventListener? onkeyup;
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
};

[Exposed=Window]
interface HTMLTextAreaElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString defaultValue;
  attribute DOMString name;
  attribute boolean disabled;
  attribute boolean readOnly;
  attribute boolean required;
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
};

[Exposed=Window]
interface HTMLIFrameElement : HTMLElement {
  readonly attribute Document? contentDocument;
};

[Exposed=Window]
interface Document : Node {
  readonly attribute DOMString URL;
  readonly attribute DOMString baseURI;
  attribute DOMString cookie;
  readonly attribute any defaultView;
  readonly attribute HTMLElement? documentElement;
  readonly attribute HTMLElement? head;
  readonly attribute HTMLElement? body;
  readonly attribute HTMLElement? activeElement;
  readonly attribute HTMLElement? currentScript;
  readonly attribute sequence<CSSStyleSheet> styleSheets;
  Element createElement(DOMString localName);
  Element createElementNS(DOMString? namespace, DOMString qualifiedName);
  Text createTextNode(DOMString data);
  Comment createComment(DOMString data);
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
  Range createRange();
  NodeIterator createNodeIterator(Node root, optional unsigned long whatToShow = 4294967295);
  TreeWalker createTreeWalker(Node root, optional unsigned long whatToShow = 4294967295);
  Selection? getSelection();
};

[Exposed=Window]
interface AbstractRange {
  readonly attribute Node startContainer;
  readonly attribute unsigned long startOffset;
  readonly attribute Node endContainer;
  readonly attribute unsigned long endOffset;
  readonly attribute boolean collapsed;
};

[Exposed=Window]
interface Range : AbstractRange {
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
  Range getRangeAt(unsigned long index);
  undefined addRange(Range range);
  undefined removeRange(Range range);
  undefined removeAllRanges();
  undefined empty();
  undefined collapse(Node? node, optional unsigned long offset = 0);
  undefined setPosition(Node? node, optional unsigned long offset = 0);
  undefined collapseToStart();
  undefined collapseToEnd();
  undefined selectAllChildren(Node node);
  undefined deleteFromDocument();
  boolean containsNode(Node node, optional boolean allowPartialContainment = false);
  DOMString toString();
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
};

[Exposed=(Window,Worker)]
interface IDBRequest : EventTarget {
  readonly attribute any result;
  readonly attribute any error;
  readonly attribute IDBTransaction? transaction;
};

[Exposed=(Window,Worker)]
interface IDBOpenDBRequest : IDBRequest {};

[Exposed=(Window,Worker)]
interface IDBDatabase : EventTarget {
  readonly attribute DOMString name;
  readonly attribute unsigned long long version;
  readonly attribute sequence<DOMString> objectStoreNames;
  IDBObjectStore createObjectStore(DOMString name, optional any options);
  undefined deleteObjectStore(DOMString name);
  IDBTransaction transaction((DOMString or sequence<DOMString>) storeNames, optional DOMString mode = "readonly");
  undefined close();
};

[Exposed=(Window,Worker)]
interface IDBTransaction : EventTarget {
  readonly attribute DOMString mode;
  readonly attribute sequence<DOMString> objectStoreNames;
  IDBObjectStore objectStore(DOMString name);
  undefined abort();
};

[Exposed=Window]
interface IDBKeyRange {
  readonly attribute any lower;
  readonly attribute any upper;
  readonly attribute boolean lowerOpen;
  readonly attribute boolean upperOpen;
};

[Exposed=Window]
interface IDBCursor {
  readonly attribute any key;
  readonly attribute any primaryKey;
  readonly attribute any value;
  undefined advance(unsigned long count);
  undefined continue(optional any key);
  undefined delete();
  undefined update(any value);
};

[Exposed=(Window,Worker)]
interface IDBIndex {
  readonly attribute DOMString name;
  readonly attribute IDBObjectStore objectStore;
  any get(any query);
  any getKey(any query);
  any getAll(optional any query, optional unsigned long count);
  any getAllKeys(optional any query, optional unsigned long count);
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
};

[Exposed=Window]
interface IDBObjectStore {
  readonly attribute DOMString name;
  readonly attribute IDBTransaction transaction;
  any add(any value, optional any key);
  any put(any value, optional any key);
  any get(any query);
  any getAll(optional any query, optional unsigned long count);
  any getAllKeys(optional any query, optional unsigned long count);
  any delete(any query);
  any clear();
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
  IDBIndex createIndex(DOMString name, (DOMString or sequence<DOMString>) keyPath, optional any options);
  IDBIndex index(DOMString name);
};

[Exposed=Window]
interface CSSStyleSheet {
  unsigned long insertRule(DOMString rule, optional unsigned long index = 0);
};

[Exposed=Window]
interface CSSStyleDeclaration {
  attribute DOMString cssText;
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
  attribute DOMString backgroundClip;
  attribute DOMString backgroundColor;
  attribute DOMString borderCollapse;
  attribute DOMString boxSizing;
  attribute DOMString cssFloat;
  attribute DOMString filter;
  attribute DOMString marginRight;
  attribute DOMString maxHeight;
  attribute DOMString minHeight;
  attribute DOMString MozBoxSizing;
  attribute DOMString WebkitBoxSizing;
  attribute DOMString zoom;
  DOMString getPropertyValue(DOMString property);
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
interface Comment : CharacterData {
};

[Exposed=Window]
interface XMLHttpRequest : EventTarget {
  readonly attribute unsigned short readyState;
  undefined open(DOMString method, DOMString url, optional boolean async = true);
  undefined send(optional DOMString? body = null);
  undefined abort();
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
interface Response {
  readonly attribute unsigned short status;
  readonly attribute boolean ok;
  Promise<DOMString> text();
  Promise<ArrayBuffer> arrayBuffer();
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
};

callback EventListener = undefined (Event event);
