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
  undefined preventDefault();
  undefined stopPropagation();
  undefined stopImmediatePropagation();
};

[Exposed=Window]
interface Node : EventTarget {
  readonly attribute DOMString nodeName;
  readonly attribute unsigned short nodeType;
  readonly attribute Node? parentNode;
  readonly attribute Node? firstChild;
  readonly attribute Node? lastChild;
  readonly attribute sequence<Node> childNodes;
  attribute DOMString? textContent;
  Node appendChild(Node child);
  Node insertBefore(Node child, Node? reference);
  Node removeChild(Node child);
  Node replaceChild(Node child, Node replaced);
  Node cloneNode(optional boolean deep = false);
  boolean contains(Node? other);
};

[Exposed=Window]
interface Element : Node {
  readonly attribute DOMString tagName;
  attribute DOMString id;
  attribute DOMString className;
  readonly attribute DOMTokenList classList;
  attribute DOMString innerHTML;
  attribute DOMString outerHTML;
  DOMString? getAttribute(DOMString name);
  undefined setAttribute(DOMString name, DOMString value);
  undefined removeAttribute(DOMString name);
  boolean hasAttribute(DOMString name);
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
  boolean matches(DOMString selectors);
  Element? closest(DOMString selectors);
  DOMRect getBoundingClientRect();
  undefined remove();
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
  undefined click();
  undefined focus();
};

[Exposed=Window]
interface HTMLInputElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString type;
  attribute DOMString name;
  attribute boolean checked;
  attribute boolean disabled;
};

[Exposed=Window]
interface HTMLTextAreaElement : HTMLElement {
  attribute DOMString value;
  attribute DOMString name;
  attribute boolean disabled;
};

[Exposed=Window]
interface HTMLSelectElement : HTMLElement {
  attribute DOMString value;
  attribute long selectedIndex;
  attribute DOMString name;
  attribute boolean disabled;
};

[Exposed=Window]
interface HTMLOptionElement : HTMLElement {
  attribute DOMString value;
  attribute boolean selected;
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
  attribute DOMString cookie;
  readonly attribute HTMLElement? documentElement;
  readonly attribute sequence<CSSStyleSheet> styleSheets;
  Element createElement(DOMString localName);
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
};

[Exposed=Window]
interface CSSStyleSheet {
  unsigned long insertRule(DOMString rule, optional unsigned long index = 0);
};

[Exposed=Window]
interface DocumentFragment : Node {
  Element? querySelector(DOMString selectors);
  sequence<Element> querySelectorAll(DOMString selectors);
};

[Exposed=Window]
interface Text : Node {
  attribute DOMString data;
};

[Exposed=Window]
interface Comment : Node {
  attribute DOMString data;
};

[Exposed=Window]
interface XMLHttpRequest : EventTarget {
  readonly attribute unsigned short readyState;
  undefined open(DOMString method, DOMString url, optional boolean async = true);
  undefined send(optional DOMString? body = null);
  undefined abort();
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
