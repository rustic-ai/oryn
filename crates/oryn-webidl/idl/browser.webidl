[Exposed=(Window,Worker)]
interface TextEncoderStream {
  constructor();
  readonly attribute DOMString encoding;
  readonly attribute ReadableStream readable;
  readonly attribute WritableStream writable;
};

[Exposed=(Window,Worker)]
interface TextDecoderStream {
  constructor(optional DOMString label = "utf-8", optional TextDecoderOptions options);
  readonly attribute DOMString encoding;
  readonly attribute boolean fatal;
  readonly attribute boolean ignoreBOM;
  readonly attribute ReadableStream readable;
  readonly attribute WritableStream writable;
};

[Exposed=(Window,Worker)]
interface TextDecoder {
  constructor(optional DOMString label = "utf-8", optional TextDecoderOptions options);
  readonly attribute DOMString encoding;
  readonly attribute boolean fatal;
  readonly attribute boolean ignoreBOM;
  USVString decode([AllowShared] optional (ArrayBuffer or ArrayBufferView) input, optional TextDecodeOptions options);
};

dictionary TextDecoderOptions {
  boolean fatal = false;
  boolean ignoreBOM = false;
};

dictionary TextDecodeOptions {
  boolean stream = false;
};

[Exposed=(Window,Worker)]
interface TextEncoder {
  constructor();
  readonly attribute DOMString encoding;
  Uint8Array encode(optional USVString input = "");
  TextEncoderEncodeIntoResult encodeInto(USVString source, [AllowShared] Uint8Array destination);
};

dictionary TextEncoderEncodeIntoResult {
  unsigned long long read;
  unsigned long long written;
};

[Global=Window, Exposed=Window]
interface Window : EventTarget {
  readonly attribute boolean isSecureContext;
  Selection? getSelection();
  [SameObject] readonly attribute CustomElementRegistry customElements;
  unsigned long requestAnimationFrame(FrameRequestCallback callback);
  undefined cancelAnimationFrame(unsigned long handle);
  [Replaceable] readonly attribute long screenX;
  [Replaceable] readonly attribute long screenY;
  [Replaceable] readonly attribute long screenLeft;
  [Replaceable] readonly attribute long screenTop;
  [Replaceable] readonly attribute long innerWidth;
  [Replaceable] readonly attribute long innerHeight;
  [Replaceable] readonly attribute long outerWidth;
  [Replaceable] readonly attribute long outerHeight;
  [SameObject] readonly attribute Navigation navigation;
  [SameObject] readonly attribute History history;
  [SameObject] readonly attribute VisualViewport? visualViewport;
  attribute DOMString name;
  readonly attribute unsigned long length;
  [LegacyUnforgeable, PutForwards=href] readonly attribute Location location;
};

[Exposed=Worker]
interface WorkerGlobalScope : EventTarget {
  readonly attribute boolean isSecureContext;
};

callback FrameRequestCallback = undefined (double time);

[Exposed=Window]
interface VisualViewport : EventTarget {
  readonly attribute double offsetLeft;
  readonly attribute double offsetTop;
  readonly attribute double pageLeft;
  readonly attribute double pageTop;
  readonly attribute double width;
  readonly attribute double height;
  readonly attribute double scale;
  attribute EventHandler onresize;
  attribute EventHandler onscroll;
  attribute EventHandler onscrollend;
};

[Exposed=(Window,Worker,AudioWorklet)]
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

[Exposed=(Window,Worker,AudioWorklet)]
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

[Exposed=Window]
interface Location {
  [LegacyUnforgeable] readonly attribute DOMStringList ancestorOrigins;
  [LegacyUnforgeable] attribute USVString href;
  [LegacyUnforgeable] readonly attribute USVString origin;
  [LegacyUnforgeable] attribute USVString protocol;
  [LegacyUnforgeable] attribute USVString host;
  [LegacyUnforgeable] attribute USVString hostname;
  [LegacyUnforgeable] attribute USVString port;
  [LegacyUnforgeable] attribute USVString pathname;
  [LegacyUnforgeable] attribute USVString search;
  [LegacyUnforgeable] attribute USVString hash;
  [LegacyUnforgeable] undefined assign(USVString url);
  [LegacyUnforgeable] undefined replace(USVString url);
  [LegacyUnforgeable] undefined reload();
  [LegacyUnforgeable] USVString toString();
};

[Exposed=(Window,Worker)]
interface URL {
  constructor(USVString url, optional USVString base);
  static URL? parse(USVString url, optional USVString base);
  static boolean canParse(USVString url, optional USVString base);
  attribute USVString href;
  readonly attribute USVString origin;
  attribute USVString protocol;
  attribute USVString username;
  attribute USVString password;
  attribute USVString host;
  attribute USVString hostname;
  attribute USVString port;
  attribute USVString pathname;
  attribute USVString search;
  attribute USVString hash;
  [SameObject] readonly attribute URLSearchParams searchParams;
  USVString toJSON();
  USVString toString();
};

[Exposed=(Window,Worker)]
interface URLSearchParams {
  constructor(optional (sequence<sequence<USVString>> or record<USVString, USVString> or USVString) init = "");
  readonly attribute unsigned long size;
  undefined append(USVString name, USVString value);
  undefined delete(USVString name, optional USVString value);
  USVString? get(USVString name);
  sequence<USVString> getAll(USVString name);
  boolean has(USVString name, optional USVString value);
  undefined set(USVString name, USVString value);
  undefined sort();
  USVString toString();
  iterable<USVString, USVString>;
};

[Exposed=(Window,Worker,AudioWorklet)]
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
  readonly attribute boolean composed;
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
interface ErrorEvent : Event {
  constructor(DOMString type, optional ErrorEventInit eventInitDict);
  readonly attribute DOMString message;
  readonly attribute USVString filename;
  readonly attribute unsigned long lineno;
  readonly attribute unsigned long colno;
  readonly attribute any error;
};

dictionary ErrorEventInit : EventInit {
  DOMString message = "";
  USVString filename = "";
  unsigned long lineno = 0;
  unsigned long colno = 0;
  any error;
};

[Exposed=(Window,Worker,AudioWorklet)]
interface MessageEvent : Event {
  constructor(DOMString type, optional MessageEventInit eventInitDict);
  readonly attribute any data;
  readonly attribute USVString origin;
  readonly attribute DOMString lastEventId;
  readonly attribute (WindowProxy or MessagePort or ServiceWorker)? source;
  readonly attribute FrozenArray<MessagePort> ports;
  readonly attribute any userActivation;
  undefined initMessageEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false, optional any data = null, optional USVString origin = "", optional DOMString lastEventId = "", optional (WindowProxy or MessagePort or ServiceWorker)? source = null, optional sequence<MessagePort> ports = []);
};

dictionary MessageEventInit : EventInit {
  any data = null;
  USVString origin = "";
  DOMString lastEventId = "";
  (WindowProxy or MessagePort or ServiceWorker)? source = null;
  sequence<MessagePort> ports = [];
};

[Exposed=(Window,Worker)]
interface CloseEvent : Event {
  constructor(DOMString type, optional CloseEventInit eventInitDict = {});
  readonly attribute boolean wasClean;
  readonly attribute unsigned short code;
  readonly attribute USVString reason;
};
dictionary CloseEventInit : EventInit {
  boolean wasClean = false;
  unsigned short code = 0;
  USVString reason = "";
};

[Exposed=(Window,Worker)]
interface ProgressEvent : Event {
  constructor(DOMString type, optional ProgressEventInit eventInitDict = {});
  readonly attribute boolean lengthComputable;
  readonly attribute unsigned long long loaded;
  readonly attribute unsigned long long total;
};

dictionary ProgressEventInit : EventInit {
  boolean lengthComputable = false;
  unsigned long long loaded = 0;
  unsigned long long total = 0;
};

[Exposed=Window]
interface BeforeUnloadEvent : Event {
  attribute DOMString returnValue;
};

[Exposed=Window]
interface PageTransitionEvent : Event {
  constructor(DOMString type, optional PageTransitionEventInit eventInitDict = {});
  readonly attribute boolean persisted;
};

dictionary PageTransitionEventInit : EventInit {
  boolean persisted = false;
};

[Exposed=Window]
interface PageSwapEvent : Event {
  constructor(DOMString type, optional PageSwapEventInit eventInitDict = {});
  readonly attribute NavigationActivation? activation;
  readonly attribute ViewTransition? viewTransition;
};

[Exposed=Window]
interface StorageEvent : Event {
  constructor(DOMString type, optional StorageEventInit eventInitDict = {});
  readonly attribute DOMString? key;
  readonly attribute DOMString? oldValue;
  readonly attribute DOMString? newValue;
  readonly attribute USVString url;
  readonly attribute Storage? storageArea;
  undefined initStorageEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false, optional DOMString? key = null, optional DOMString? oldValue = null, optional DOMString? newValue = null, optional USVString url = "", optional Storage? storageArea = null);
};
dictionary StorageEventInit : EventInit {
  DOMString? key = null;
  DOMString? oldValue = null;
  DOMString? newValue = null;
  USVString url = "";
  Storage? storageArea = null;
};

[Exposed=Window]
interface PopStateEvent : Event {
  constructor(DOMString type, optional PopStateEventInit eventInitDict = {});
  readonly attribute any state;
  readonly attribute boolean hasUAVisualTransition;
};
dictionary PopStateEventInit : EventInit {
  any state = null;
  boolean hasUAVisualTransition = false;
};

[Exposed=Window]
interface HashChangeEvent : Event {
  constructor(DOMString type, optional HashChangeEventInit eventInitDict = {});
  readonly attribute USVString oldURL;
  readonly attribute USVString newURL;
};
dictionary HashChangeEventInit : EventInit {
  USVString oldURL = "";
  USVString newURL = "";
};

dictionary PageSwapEventInit : EventInit {
  NavigationActivation? activation = null;
  ViewTransition? viewTransition = null;
};

[Exposed=Window]
interface PageRevealEvent : Event {
  constructor(DOMString type, optional PageRevealEventInit eventInitDict = {});
  readonly attribute ViewTransition? viewTransition;
};

dictionary PageRevealEventInit : EventInit {
  ViewTransition? viewTransition = null;
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

[Exposed=Window]
interface SubmitEvent : Event {
  constructor(DOMString type, optional SubmitEventInit eventInitDict);
  readonly attribute HTMLElement? submitter;
};

dictionary SubmitEventInit : EventInit {
  HTMLElement? submitter = null;
};

[Exposed=Window]
interface FormDataEvent : Event {
  constructor(DOMString type, FormDataEventInit eventInitDict);
  readonly attribute FormData formData;
};

dictionary FormDataEventInit : EventInit {
  required FormData formData;
};

// UI Events: https://www.w3.org/TR/uievents/#interface-focusevent
[Exposed=Window]
interface UIEvent : Event {
  constructor(DOMString type, optional UIEventInit eventInitDict);
  readonly attribute Window? view;
  readonly attribute long detail;
  undefined initUIEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false, optional Window? view = null, optional long detail = 0);
};

dictionary UIEventInit : EventInit {
  Window? view = null;
  long detail = 0;
};

[Exposed=Window]
interface FocusEvent : UIEvent {
  constructor(DOMString type, optional FocusEventInit eventInitDict);
  readonly attribute EventTarget? relatedTarget;
};

dictionary FocusEventInit : UIEventInit {
  EventTarget? relatedTarget = null;
};

dictionary EventModifierInit : UIEventInit {
  boolean ctrlKey = false;
  boolean shiftKey = false;
  boolean altKey = false;
  boolean metaKey = false;
  boolean modifierAltGraph = false;
  boolean modifierCapsLock = false;
};

dictionary MouseEventInit : EventModifierInit {
  long screenX = 0;
  long screenY = 0;
  long clientX = 0;
  long clientY = 0;
  short button = 0;
  unsigned short buttons = 0;
  EventTarget? relatedTarget = null;
};

[Exposed=Window]
interface MouseEvent : UIEvent {
  constructor(DOMString type, optional MouseEventInit eventInitDict);
  readonly attribute long screenX;
  readonly attribute long screenY;
  readonly attribute long clientX;
  readonly attribute long clientY;
  readonly attribute boolean ctrlKey;
  readonly attribute boolean shiftKey;
  readonly attribute boolean altKey;
  readonly attribute boolean metaKey;
  readonly attribute short button;
  readonly attribute unsigned short buttons;
  readonly attribute EventTarget? relatedTarget;
  boolean getModifierState(DOMString keyArg);
};

dictionary PointerEventInit : MouseEventInit {
  long pointerId = 0;
  double width = 1;
  double height = 1;
  float pressure = 0;
  float tangentialPressure = 0;
  long tiltX = 0;
  long tiltY = 0;
  long twist = 0;
  DOMString pointerType = "";
  boolean isPrimary = false;
};

[Exposed=Window]
interface PointerEvent : MouseEvent {
  constructor(DOMString type, optional PointerEventInit eventInitDict);
  readonly attribute long pointerId;
  readonly attribute double width;
  readonly attribute double height;
  readonly attribute float pressure;
  readonly attribute float tangentialPressure;
  readonly attribute long tiltX;
  readonly attribute long tiltY;
  readonly attribute long twist;
  readonly attribute DOMString pointerType;
  readonly attribute boolean isPrimary;
};

dictionary KeyboardEventInit : EventModifierInit {
  DOMString key = "";
  DOMString code = "";
  unsigned long location = 0;
  boolean repeat = false;
  boolean isComposing = false;
  unsigned long charCode = 0;
  unsigned long keyCode = 0;
};

[Exposed=Window]
interface KeyboardEvent : UIEvent {
  constructor(DOMString type, optional KeyboardEventInit eventInitDict);
  readonly attribute DOMString key;
  readonly attribute DOMString code;
  readonly attribute unsigned long location;
  readonly attribute boolean ctrlKey;
  readonly attribute boolean shiftKey;
  readonly attribute boolean altKey;
  readonly attribute boolean metaKey;
  readonly attribute boolean repeat;
  readonly attribute boolean isComposing;
  readonly attribute unsigned long charCode;
  readonly attribute unsigned long keyCode;
  boolean getModifierState(DOMString keyArg);
};

dictionary WheelEventInit : MouseEventInit {
  double deltaX = 0;
  double deltaY = 0;
  double deltaZ = 0;
  unsigned long deltaMode = 0;
};

[Exposed=Window]
interface WheelEvent : MouseEvent {
  constructor(DOMString type, optional WheelEventInit eventInitDict);
  readonly attribute double deltaX;
  readonly attribute double deltaY;
  readonly attribute double deltaZ;
  readonly attribute unsigned long deltaMode;
};

dictionary TouchInit {
  required long identifier;
  required EventTarget target;
  double clientX = 0;
  double clientY = 0;
  double screenX = 0;
  double screenY = 0;
  double pageX = 0;
  double pageY = 0;
  float radiusX = 0;
  float radiusY = 0;
  float rotationAngle = 0;
  float force = 0;
};

[Exposed=Window]
interface Touch {
  constructor(TouchInit touchInitDict);
  readonly attribute long identifier;
  readonly attribute EventTarget target;
  readonly attribute double screenX;
  readonly attribute double screenY;
  readonly attribute double clientX;
  readonly attribute double clientY;
  readonly attribute double pageX;
  readonly attribute double pageY;
  readonly attribute float radiusX;
  readonly attribute float radiusY;
  readonly attribute float rotationAngle;
  readonly attribute float force;
};

[Exposed=Window]
interface TouchList {
  readonly attribute unsigned long length;
  getter Touch? item(unsigned long index);
};

dictionary TouchEventInit : EventModifierInit {
  sequence<Touch> touches = [];
  sequence<Touch> targetTouches = [];
  sequence<Touch> changedTouches = [];
};

[Exposed=Window]
interface TouchEvent : UIEvent {
  constructor(DOMString type, optional TouchEventInit eventInitDict);
  readonly attribute TouchList touches;
  readonly attribute TouchList targetTouches;
  readonly attribute TouchList changedTouches;
  readonly attribute boolean altKey;
  readonly attribute boolean metaKey;
  readonly attribute boolean ctrlKey;
  readonly attribute boolean shiftKey;
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

[Exposed=(Window,Worker,AudioWorklet)]
interface MessagePort : EventTarget {
  attribute EventListener? onmessage;
  undefined postMessage(any message, optional sequence<object> transfer = []);
  undefined start();
  undefined close();
  attribute EventListener? onmessageerror;
  attribute EventListener? onclose;
};

[Exposed=(Window,Worker)]
interface MessageChannel {
  constructor();
  readonly attribute MessagePort port1;
  readonly attribute MessagePort port2;
};

[Exposed=(Window,Worker)]
interface BroadcastChannel : EventTarget {
  constructor(DOMString name);
  readonly attribute DOMString name;
  undefined postMessage(any message);
  undefined close();
  attribute EventListener? onmessage;
  attribute EventListener? onmessageerror;
};

callback MutationCallback = undefined (sequence<MutationRecord> mutations, MutationObserver observer);

dictionary MutationObserverInit {
  boolean childList = false;
  boolean attributes;
  boolean characterData;
  boolean subtree = false;
  boolean attributeOldValue;
  boolean characterDataOldValue;
  sequence<DOMString> attributeFilter;
};

[Exposed=Window]
interface MutationRecord {
  readonly attribute DOMString type;
  readonly attribute Node target;
  readonly attribute NodeList addedNodes;
  readonly attribute NodeList removedNodes;
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
  USVString documentURI = "";
  USVString referrer = "";
  USVString blockedURI = "";
  DOMString violatedDirective = "";
  DOMString effectiveDirective = "";
  DOMString originalPolicy = "";
  USVString sourceFile = "";
  DOMString sample = "";
  SecurityPolicyViolationEventDisposition disposition = "enforce";
  unsigned short statusCode = 0;
  unsigned long lineNumber = 0;
  unsigned long columnNumber = 0;
};

[Exposed=(Window,Worker)]
interface SecurityPolicyViolationEvent : Event {
  constructor(DOMString type, optional SecurityPolicyViolationEventInit eventInitDict);
  readonly attribute USVString documentURI;
  readonly attribute USVString referrer;
  readonly attribute USVString blockedURI;
  readonly attribute DOMString violatedDirective;
  readonly attribute DOMString effectiveDirective;
  readonly attribute DOMString originalPolicy;
  readonly attribute USVString sourceFile;
  readonly attribute DOMString sample;
  readonly attribute SecurityPolicyViolationEventDisposition disposition;
  readonly attribute unsigned short statusCode;
  readonly attribute unsigned long lineNumber;
  readonly attribute unsigned long columnNumber;
};

[Exposed=Window, LegacyUnenumerableNamedProperties]
interface HTMLAllCollection {
  readonly attribute unsigned long length;
  getter Element (unsigned long index);
  getter (HTMLCollection or Element)? namedItem(DOMString name);
  (HTMLCollection or Element)? item(optional DOMString nameOrIndex);
};

[Exposed=Window, LegacyUnenumerableNamedProperties]
interface HTMLCollection {
  readonly attribute unsigned long length;
  getter Element? item(unsigned long index);
  getter Element? namedItem(DOMString name);
};

[Exposed=Window]
interface HTMLOptionsCollection : HTMLCollection {
  [CEReactions] setter undefined (unsigned long index, HTMLOptionElement? option);
  [CEReactions] attribute unsigned long length;
  [CEReactions] undefined add((HTMLOptionElement or HTMLOptGroupElement) element, optional (HTMLElement or long)? before = null);
  [CEReactions] undefined remove(long index);
  attribute long selectedIndex;
};

[Exposed=Window]
interface NodeList {
  getter Node? item(unsigned long index);
  readonly attribute unsigned long length;
  iterable<Node>;
};

[Exposed=Window, LegacyUnenumerableNamedProperties]
interface HTMLFormControlsCollection : HTMLCollection {
  getter (RadioNodeList or Element)? namedItem(DOMString name);
};

[Exposed=Window]
interface RadioNodeList : NodeList {
  attribute DOMString value;
};

[Exposed=Window]
interface Node : EventTarget {
  DOMString? lookupPrefix(DOMString? namespace);
  DOMString? lookupNamespaceURI(DOMString? prefix);
  boolean isDefaultNamespace(DOMString? namespace);
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
  readonly attribute DOMString baseURI;
  readonly attribute Document? ownerDocument;
  readonly attribute Node? parentNode;
  readonly attribute Element? parentElement;
  readonly attribute Node? firstChild;
  readonly attribute Node? lastChild;
  readonly attribute Node? previousSibling;
  readonly attribute Node? nextSibling;
  [SameObject] readonly attribute NodeList childNodes;
  readonly attribute boolean isConnected;
  [CEReactions] attribute DOMString? nodeValue;
  [CEReactions] attribute DOMString? textContent;
  [CEReactions] Node appendChild(Node child);
  [CEReactions] Node insertBefore(Node child, Node? reference);
  [CEReactions] Node removeChild(Node child);
  [CEReactions] Node replaceChild(Node child, Node replaced);
  [CEReactions] undefined normalize();
  [CEReactions] Node cloneNode(optional boolean deep = false);
  boolean contains(Node? other);
  boolean isEqualNode(Node? other);
  boolean isSameNode(Node? otherNode);
  unsigned short compareDocumentPosition(Node other);
  boolean hasChildNodes();
  Node getRootNode(optional GetRootNodeOptions options = {});
};

[Exposed=Window]
interface CustomElementRegistry {
  constructor();
  [CEReactions] undefined define(DOMString name, CustomElementConstructor constructor, optional ElementDefinitionOptions options = {});
  (CustomElementConstructor or undefined) get(DOMString name);
  DOMString? getName(CustomElementConstructor constructor);
  Promise<CustomElementConstructor> whenDefined(DOMString name);
  [CEReactions] undefined upgrade(Node root);
  [CEReactions] undefined initialize(Node root);
};
callback CustomElementConstructor = HTMLElement ();
dictionary ElementDefinitionOptions { DOMString extends; };
dictionary ElementCreationOptions { CustomElementRegistry? customElementRegistry; DOMString is; };
dictionary ImportNodeOptions { CustomElementRegistry customElementRegistry; boolean selfOnly = false; };

enum ShadowRootMode { "open", "closed" };
enum SlotAssignmentMode { "manual", "named" };
[Exposed=Window]
interface Sanitizer {
  constructor(optional (SanitizerConfig or SanitizerPresets) configuration = "default");

  // Query configuration:
  SanitizerConfig get();

  // Modify a Sanitizer's lists and fields:
  boolean allowElement(SanitizerElementWithAttributes element);
  boolean removeElement(SanitizerElement element);
  boolean replaceElementWithChildren(SanitizerElement element);
  boolean allowProcessingInstruction(SanitizerPI pi);
  boolean removeProcessingInstruction(SanitizerPI pi);
  boolean allowAttribute(SanitizerAttribute attribute);
  boolean removeAttribute(SanitizerAttribute attribute);
  boolean setComments(boolean allow);
  boolean setDataAttributes(boolean allow);

  // Remove markup that executes script.
  boolean removeUnsafe();
};

dictionary SanitizerElementNamespace {
  required DOMString name;
  DOMString? _namespace = "http://www.w3.org/1999/xhtml";
};

// Used by "elements"
dictionary SanitizerElementNamespaceWithAttributes : SanitizerElementNamespace {
  sequence<SanitizerAttribute> attributes;
  sequence<SanitizerAttribute> removeAttributes;
};

dictionary SanitizerAttributeNamespace {
  required DOMString name;
  DOMString? _namespace = null;
};

dictionary SanitizerProcessingInstruction {
  required DOMString target;
};

typedef (DOMString or SanitizerElementNamespace) SanitizerElement;
typedef (DOMString or SanitizerElementNamespaceWithAttributes) SanitizerElementWithAttributes;
typedef (DOMString or SanitizerProcessingInstruction) SanitizerPI;
typedef (DOMString or SanitizerAttributeNamespace) SanitizerAttribute;

dictionary SanitizerConfig {
  sequence<SanitizerElementWithAttributes> elements;
  sequence<SanitizerElement> removeElements;
  sequence<SanitizerElement> replaceWithChildrenElements;

  sequence<SanitizerPI> processingInstructions;
  sequence<SanitizerPI> removeProcessingInstructions;

  sequence<SanitizerAttribute> attributes;
  sequence<SanitizerAttribute> removeAttributes;

  boolean comments;
  boolean dataAttributes;
};

enum SanitizerPresets { "default" };
dictionary SetHTMLOptions {
  (Sanitizer or SanitizerConfig or SanitizerPresets) sanitizer = "default";
};
dictionary SetHTMLUnsafeOptions {
  (Sanitizer or SanitizerConfig or SanitizerPresets) sanitizer = {};
  boolean runScripts = false;
};
dictionary ParseHTMLUnsafeOptions {
  (Sanitizer or SanitizerConfig or SanitizerPresets) sanitizer = {};
};

dictionary GetHTMLOptions {
  boolean serializableShadowRoots = false;
  sequence<ShadowRoot> shadowRoots = [];
};

dictionary ShadowRootInit {
  CustomElementRegistry? customElementRegistry;
  boolean clonable = false;
  boolean delegatesFocus = false;
  required ShadowRootMode mode;
  DOMString? referenceTarget = null;
  boolean serializable = false;
  SlotAssignmentMode slotAssignment = "named";
};

[Exposed=Window]
interface Element : Node {
  WritableStream streamBeforeHTML(optional SetHTMLOptions options = {});
  WritableStream streamBeforeHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamAfterHTML(optional SetHTMLOptions options = {});
  WritableStream streamAfterHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamReplaceWithHTML(optional SetHTMLOptions options = {});
  WritableStream streamReplaceWithHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamHTML(optional SetHTMLOptions options = {});
  WritableStream streamHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamAppendHTML(optional SetHTMLOptions options = {});
  WritableStream streamAppendHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamPrependHTML(optional SetHTMLOptions options = {});
  WritableStream streamPrependHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  [CEReactions] undefined appendHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined appendHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined prependHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined prependHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined beforeHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined beforeHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined afterHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined afterHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined replaceWithHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined replaceWithHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined setHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined setHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  DOMString getHTML(optional GetHTMLOptions options = {});
  readonly attribute CustomElementRegistry? customElementRegistry;
  [CEReactions, Unscopable] undefined before((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined after((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceWith((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined remove();

  [CEReactions, Unscopable] undefined append((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined prepend((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceChildren((Node or DOMString)... nodes);
  [CEReactions] undefined moveBefore(Node movedNode, Node? referenceNode);

  readonly attribute HTMLSlotElement? assignedSlot;
  readonly attribute DOMString tagName;
  readonly attribute DOMString localName;
  readonly attribute DOMString? namespaceURI;
  readonly attribute DOMString? prefix;
  [SameObject] readonly attribute NamedNodeMap attributes;
  [SameObject] readonly attribute HTMLCollection children;
  readonly attribute unsigned long childElementCount;
  readonly attribute Element? firstElementChild;
  readonly attribute Element? lastElementChild;
  readonly attribute Element? previousElementSibling;
  readonly attribute Element? nextElementSibling;
  [CEReactions] attribute DOMString id;
  [CEReactions] attribute DOMString className;
  [CEReactions] attribute DOMString slot;
  [SameObject, PutForwards=value] readonly attribute DOMTokenList classList;
  [CEReactions] attribute DOMString innerHTML;
  [CEReactions] attribute DOMString outerHTML;
  readonly attribute ShadowRoot? shadowRoot;
  ShadowRoot attachShadow(ShadowRootInit init);
  DOMString? getAttribute(DOMString name);
  DOMString? getAttributeNS(DOMString? namespace, DOMString localName);
  Attr? getAttributeNode(DOMString name);
  Attr? getAttributeNodeNS(DOMString? namespace, DOMString localName);
  [CEReactions] Attr? setAttributeNode(Attr attr);
  [CEReactions] Attr? setAttributeNodeNS(Attr attr);
  [CEReactions] Attr removeAttributeNode(Attr attr);
  sequence<DOMString> getAttributeNames();
  [CEReactions] undefined setAttribute(DOMString name, DOMString value);
  [CEReactions] undefined setAttributeNS(DOMString? namespace, DOMString qualifiedName, DOMString value);
  [CEReactions] undefined removeAttribute(DOMString name);
  [CEReactions] undefined removeAttributeNS(DOMString? namespace, DOMString localName);
  boolean hasAttribute(DOMString name);
  boolean hasAttributeNS(DOMString? namespace, DOMString localName);
  boolean hasAttributes();
  undefined setPointerCapture(long pointerId);
  undefined releasePointerCapture(long pointerId);
  boolean hasPointerCapture(long pointerId);
  [CEReactions] boolean toggleAttribute(DOMString name, optional boolean force);
  Element? querySelector(DOMString selectors);
  [NewObject] NodeList querySelectorAll(DOMString selectors);
  HTMLCollection getElementsByTagName(DOMString qualifiedName);
  HTMLCollection getElementsByTagNameNS(DOMString? namespace, DOMString localName);
  HTMLCollection getElementsByClassName(DOMString classNames);
  boolean matches(DOMString selectors);
  boolean webkitMatchesSelector(DOMString selectors);
  Element? closest(DOMString selectors);
  [CEReactions] Element? insertAdjacentElement(DOMString where, Element element);
  undefined insertAdjacentText(DOMString where, DOMString data);
  [CEReactions] undefined insertAdjacentHTML(DOMString where, DOMString html);
  DOMRect getBoundingClientRect();
  DOMRectList getClientRects();
  attribute unrestricted double scrollLeft;
  attribute unrestricted double scrollTop;
  readonly attribute long scrollWidth;
  readonly attribute long scrollHeight;
  readonly attribute long clientTop;
  readonly attribute long clientLeft;
  readonly attribute long clientWidth;
  readonly attribute long clientHeight;
  undefined scroll(unrestricted double x, unrestricted double y);
  undefined scroll(optional ScrollToOptions options = {});
  undefined scrollTo(unrestricted double x, unrestricted double y);
  undefined scrollTo(optional ScrollToOptions options = {});
  undefined scrollBy(unrestricted double x, unrestricted double y);
  undefined scrollBy(optional ScrollToOptions options = {});
  undefined scrollIntoView(optional boolean alignToTop = true);
  [Unscopable] undefined remove();
};

enum ScrollBehavior { "auto", "instant", "smooth" };
dictionary ScrollOptions {
  ScrollBehavior behavior = "auto";
};
dictionary ScrollToOptions : ScrollOptions {
  unrestricted double left;
  unrestricted double top;
};

[Exposed=Window]
interface Attr : Node {
  readonly attribute DOMString name;
  readonly attribute DOMString localName;
  readonly attribute DOMString? namespaceURI;
  readonly attribute DOMString? prefix;
  readonly attribute boolean specified;
  readonly attribute boolean expando;
  [CEReactions] attribute DOMString value;
  readonly attribute Element? ownerElement;
};

[Exposed=Window, LegacyUnenumerableNamedProperties]
interface NamedNodeMap {
  readonly attribute unsigned long length;
  getter Attr? item(unsigned long index);
  getter Attr? getNamedItem(DOMString qualifiedName);
  Attr? getNamedItemNS(DOMString? namespace, DOMString localName);
  [CEReactions] Attr? setNamedItem(Attr attr);
  [CEReactions] Attr? setNamedItemNS(Attr attr);
  [CEReactions] Attr removeNamedItem(DOMString qualifiedName);
  [CEReactions] Attr removeNamedItemNS(DOMString? namespace, DOMString localName);
};

[Exposed=Window]
interface DocumentType : Node {
  WritableStream streamBeforeHTML(optional SetHTMLOptions options = {});
  WritableStream streamBeforeHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamAfterHTML(optional SetHTMLOptions options = {});
  WritableStream streamAfterHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamReplaceWithHTML(optional SetHTMLOptions options = {});
  WritableStream streamReplaceWithHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  [CEReactions, Unscopable] undefined before((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined after((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceWith((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined remove();

  readonly attribute DOMString name;
  readonly attribute DOMString publicId;
  readonly attribute DOMString systemId;
};

[Exposed=Window]
interface SVGElement : Element {
  [CEReactions] attribute boolean autofocus;
  attribute DOMString nonce;
  attribute EventListener? ondragstart;
  attribute EventListener? ondrag;
  attribute EventListener? ondragenter;
  attribute EventListener? ondragleave;
  attribute EventListener? ondragover;
  attribute EventListener? ondrop;
  attribute EventListener? ondragend;
  attribute EventListener? oncopy;
  attribute EventListener? oncut;
  attribute EventListener? onpaste;

  [CEReactions] attribute long tabIndex;
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleDeclaration style;
};

[Exposed=Window]
interface SVGAElement : SVGElement {
};

[Exposed=Window]
interface HTMLDivElement : HTMLElement {
  [CEReactions] attribute DOMString align;
};

[Exposed=Window]
interface HTMLSpanElement : HTMLElement {};

[Exposed=Window]
interface HTMLUnknownElement : HTMLElement {};

[Exposed=Window]
interface HTMLHtmlElement : HTMLElement {};

[Exposed=Window]
interface HTMLHeadElement : HTMLElement {};

[Exposed=Window]
interface HTMLTitleElement : HTMLElement {};

[Exposed=Window]
interface HTMLBaseElement : HTMLElement {
  [CEReactions] attribute USVString href;
  [CEReactions] attribute DOMString target;
};

[Exposed=Window]
interface HTMLBodyElement : HTMLElement {
  [CEReactions] attribute USVString background;
  attribute EventListener? onpageshow;
  attribute EventListener? onpagehide;
  attribute EventListener? onpageswap;
  attribute EventListener? onpagereveal;
  attribute EventListener? onpopstate;
  attribute EventListener? onhashchange;
  attribute EventListener? onstorage;
};

[Exposed=Window]
interface HTMLFrameSetElement : HTMLElement {
  [CEReactions, Reflect] attribute DOMString cols;
  [CEReactions, Reflect] attribute DOMString rows;
  attribute EventListener? onpageshow;
  attribute EventListener? onpagehide;
  attribute EventListener? onpageswap;
  attribute EventListener? onpagereveal;
  attribute EventListener? onpopstate;
  attribute EventListener? onhashchange;
  attribute EventListener? onstorage;
};

[Exposed=Window]
interface HTMLMetaElement : HTMLElement {
  [CEReactions, Reflect] attribute DOMString name;
  [CEReactions, Reflect] attribute DOMString content;
  [CEReactions, Reflect] attribute DOMString httpEquiv;
};

[Exposed=Window]
interface DOMRectList {
  readonly attribute unsigned long length;
  getter DOMRect? item(unsigned long index);
};

[Exposed=(Window,Worker)]
interface DOMRect : DOMRectReadOnly {
  constructor(optional unrestricted double x = 0, optional unrestricted double y = 0, optional unrestricted double width = 0, optional unrestricted double height = 0);
  attribute unrestricted double x;
  attribute unrestricted double y;
  attribute unrestricted double width;
  attribute unrestricted double height;
};

[Exposed=(Window,Worker)]
interface DOMRectReadOnly {
  constructor(optional unrestricted double x = 0, optional unrestricted double y = 0, optional unrestricted double width = 0, optional unrestricted double height = 0);
  readonly attribute unrestricted double x;
  readonly attribute unrestricted double y;
  readonly attribute unrestricted double width;
  readonly attribute unrestricted double height;
  readonly attribute unrestricted double top;
  readonly attribute unrestricted double right;
  readonly attribute unrestricted double bottom;
  readonly attribute unrestricted double left;
};

[Exposed=(Window,Worker)]
interface DOMQuad {
  constructor(optional any p1, optional any p2, optional any p3, optional any p4);
  readonly attribute DOMPoint p1;
  readonly attribute DOMPoint p2;
  readonly attribute DOMPoint p3;
  readonly attribute DOMPoint p4;
  DOMRect getBounds();
  object toJSON();
};

[Exposed=(Window,Worker)]
interface DOMPoint : DOMPointReadOnly {
  constructor(optional unrestricted double x = 0, optional unrestricted double y = 0, optional unrestricted double z = 0, optional unrestricted double w = 1);
};

[Exposed=(Window,Worker)]
interface DOMPointReadOnly {
  constructor(optional unrestricted double x = 0, optional unrestricted double y = 0, optional unrestricted double z = 0, optional unrestricted double w = 1);
};

[Exposed=(Window,Worker)]
interface DOMMatrix : DOMMatrixReadOnly {
  constructor(optional any init = null);
};

[Exposed=(Window,Worker)]
interface DOMMatrixReadOnly {
  constructor(optional any init = null);
};

[Exposed=(Window,DedicatedWorker), Serializable, Transferable]
interface AudioData {
  constructor(any init);
  readonly attribute DOMString? format;
  readonly attribute float sampleRate;
  readonly attribute unsigned long numberOfFrames;
  readonly attribute unsigned long numberOfChannels;
  readonly attribute unsigned long long duration;
  readonly attribute long long timestamp;
  unsigned long allocationSize(any options);
  undefined copyTo(any destination, any options);
  AudioData clone();
  undefined close();
};

[Exposed=(Window,DedicatedWorker), Serializable, Transferable]
interface VideoFrame {
  constructor(any image, optional any init = null);
  readonly attribute DOMString? format;
  readonly attribute unsigned long codedWidth;
  readonly attribute unsigned long codedHeight;
  readonly attribute DOMRectReadOnly? codedRect;
  readonly attribute DOMRectReadOnly? visibleRect;
  readonly attribute unsigned long displayWidth;
  readonly attribute unsigned long displayHeight;
  readonly attribute unsigned long long? duration;
  readonly attribute long long timestamp;
  readonly attribute double rotation;
  readonly attribute boolean flip;
  VideoFrame clone();
  undefined close();
  object metadata();
};

[Exposed=(Window,Worker), Serializable, Transferable]
interface ImageBitmap {
  readonly attribute unsigned long width;
  readonly attribute unsigned long height;
  undefined close();
};

[Exposed=(Window,Worker)]
interface ImageData {
  constructor(unsigned long sw, unsigned long sh);
  constructor(Uint8ClampedArray data, unsigned long sw, optional unsigned long sh);
  readonly attribute unsigned long width;
  readonly attribute unsigned long height;
  readonly attribute Uint8ClampedArray data;
};

[Exposed=Window, LegacyOverrideBuiltIns]
interface DOMStringMap {
  getter DOMString (DOMString name);
  [CEReactions] setter undefined (DOMString name, DOMString value);
  [CEReactions] deleter undefined (DOMString name);
};

[Exposed=Window]
interface DOMTokenList {
  readonly attribute unsigned long length;
  getter DOMString? item(unsigned long index);
  [CEReactions] attribute DOMString value;
  DOMString toString();
  [CEReactions] undefined add(DOMString... tokens);
  [CEReactions] undefined remove(DOMString... tokens);
  [CEReactions] boolean toggle(DOMString token, optional boolean force);
  boolean contains(DOMString token);
  [CEReactions] boolean replace(DOMString token, DOMString newToken);
  boolean supports(DOMString token);
  iterable<DOMString>;
};

[Exposed=Window]
interface HTMLElement : Element {
  [CEReactions] attribute boolean autofocus;
  attribute DOMString nonce;
  [CEReactions] attribute DOMString? popover;
  attribute EventListener? oncommand;
  attribute EventListener? onbeforetoggle;
  attribute EventListener? ontoggle;
  undefined showPopover(optional ShowPopoverOptions options = {});
  undefined hidePopover();
  boolean togglePopover(optional (boolean or TogglePopoverOptions) options = {});
  attribute EventListener? ondragstart;
  attribute EventListener? ondrag;
  attribute EventListener? ondragenter;
  attribute EventListener? ondragleave;
  attribute EventListener? ondragover;
  attribute EventListener? ondrop;
  attribute EventListener? ondragend;
  attribute EventListener? oncopy;
  attribute EventListener? oncut;
  attribute EventListener? onpaste;

  [CEReactions] attribute long tabIndex;
  readonly attribute long offsetWidth;
  readonly attribute long offsetHeight;
  [CEReactions] attribute DOMString title;
  [CEReactions] attribute DOMString lang;
  [CEReactions] attribute DOMString dir;
  [CEReactions] attribute DOMString accessKey;
  [CEReactions] attribute boolean translate;
  [CEReactions] attribute boolean spellcheck;
  [CEReactions] attribute boolean hidden;
  [CEReactions] attribute boolean inert;
  [CEReactions] attribute DOMString innerText;
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleDeclaration style;
  [SameObject] readonly attribute DOMStringMap dataset;
  attribute EventListener? onclick;
  attribute EventListener? oninput;
  attribute EventListener? onchange;
  attribute EventListener? onsubmit;
  attribute EventListener? onformdata;
  attribute EventListener? onfocus;
  attribute EventListener? onblur;
  attribute EventListener? onkeydown;
  attribute EventListener? onkeyup;
  attribute EventListener? onload;
  attribute EventListener? onerror;
  [CEReactions] attribute boolean draggable;
  attribute DOMString contentEditable;
  readonly attribute boolean isContentEditable;
  undefined click();
  undefined focus();
  undefined blur();
  ElementInternals attachInternals(optional ElementInternalsOptions options = {});
};

[Exposed=Window]
interface HTMLSlotElement : HTMLElement {
  undefined assign(Node... nodes);
  [CEReactions] attribute DOMString name;
  sequence<Node> assignedNodes(optional AssignedNodesOptions options = {});
  sequence<Element> assignedElements(optional AssignedNodesOptions options = {});
};

dictionary AssignedNodesOptions {
  boolean flatten = false;
};

[Exposed=Window]
interface ElementInternals {
  readonly attribute FrozenArray<HTMLSubmitButtonBehavior> behaviors;
  readonly attribute HTMLFormElement? form;
  undefined setFormValue((File or USVString or FormData)? value, optional (File or USVString or FormData)? state);
  undefined setValidity(optional ValidityStateFlags flags = {}, optional DOMString message, optional HTMLElement anchor);
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  boolean reportValidity();
  readonly attribute NodeList labels;
  readonly attribute ShadowRoot? shadowRoot;
  readonly attribute CustomStateSet states;
};

[Exposed=Window]
interface CustomStateSet {
  iterable<DOMString>;
  any keys();
  any values();
  any entries();
  undefined forEach(any callback, optional any thisArg);
  readonly attribute unsigned long size;
  CustomStateSet add(DOMString state);
  boolean delete(DOMString state);
  boolean has(DOMString state);
  undefined clear();
};

[Exposed=Window]
interface HTMLTemplateElement : HTMLElement {
  [CEReactions, Reflect] attribute boolean buffer;
  [CEReactions, Reflect=shadowrootadoptedstylesheets] attribute DOMString shadowRootAdoptedStyleSheets;
  readonly attribute DocumentFragment content;
};

[Exposed=Window]
interface HTMLMediaElement : HTMLElement {
  [SameObject] readonly attribute AudioTrackList audioTracks;
  [SameObject] readonly attribute VideoTrackList videoTracks;
  [SameObject] readonly attribute TextTrackList textTracks;
  TextTrack addTextTrack(DOMString kind, optional DOMString label = "", optional DOMString language = "");
  [CEReactions] attribute USVString src;
  [CEReactions] attribute DOMString preload;
  [CEReactions] attribute DOMString? crossOrigin;
  readonly attribute USVString currentSrc;
  readonly attribute unsigned short networkState;
  readonly attribute unrestricted double duration;
  undefined load();
  readonly attribute MediaError? error;
  attribute MediaStream? srcObject;
  attribute boolean muted;
  attribute double volume;
  readonly attribute boolean paused;
  readonly attribute boolean ended;
  readonly attribute unsigned short readyState;
  attribute double currentTime;
  readonly attribute boolean seeking;
  [CEReactions] attribute boolean loop;
  [CEReactions] attribute boolean autoplay;
  DOMString canPlayType(DOMString type);
  Promise<undefined> play();
  undefined pause();
  attribute EventListener? onloadstart;
  attribute EventListener? onprogress;
  attribute EventListener? onsuspend;
  attribute EventListener? onabort;
  attribute EventListener? onemptied;
  attribute EventListener? onstalled;
  attribute EventListener? onloadedmetadata;
  attribute EventListener? onloadeddata;
  attribute EventListener? oncanplay;
  attribute EventListener? oncanplaythrough;
  attribute EventListener? onplaying;
  attribute EventListener? onwaiting;
  attribute EventListener? onseeking;
  attribute EventListener? onseeked;
  attribute EventListener? onended;
  attribute EventListener? ondurationchange;
  attribute EventListener? ontimeupdate;
  attribute EventListener? onplay;
  attribute EventListener? onpause;
  attribute EventListener? onratechange;
  attribute EventListener? onvolumechange;
};
[Exposed=Window]
interface HTMLVideoElement : HTMLMediaElement {
  [CEReactions] attribute USVString poster;
  readonly attribute unsigned long videoWidth;
  readonly attribute unsigned long videoHeight;
};
[Exposed=Window]
interface HTMLAudioElement : HTMLMediaElement {};

[Exposed=Window]
interface HTMLCanvasElement : HTMLElement {
  [CEReactions] attribute unsigned long width;
  [CEReactions] attribute unsigned long height;
  DOMString toDataURL(optional DOMString type = "image/png", optional any quality);
  undefined toBlob(any callback, optional DOMString type = "image/png", optional any quality);
  any getContext(DOMString contextId);
};

[Exposed=(Window,Worker)]
interface Path2D {
  constructor(optional any path);
  undefined addPath(Path2D path, optional any transform);
  undefined closePath();
  undefined moveTo(unrestricted double x, unrestricted double y);
  undefined lineTo(unrestricted double x, unrestricted double y);
  undefined rect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined roundRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h, optional any radii);
  undefined quadraticCurveTo(unrestricted double cpx, unrestricted double cpy, unrestricted double x, unrestricted double y);
  undefined bezierCurveTo(unrestricted double cp1x, unrestricted double cp1y, unrestricted double cp2x, unrestricted double cp2y, unrestricted double x, unrestricted double y);
  undefined arc(unrestricted double x, unrestricted double y, unrestricted double radius, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
  undefined arcTo(unrestricted double x1, unrestricted double y1, unrestricted double x2, unrestricted double y2, unrestricted double radius);
  undefined ellipse(unrestricted double x, unrestricted double y, unrestricted double radiusX, unrestricted double radiusY, unrestricted double rotation, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
};

[Exposed=Window]
interface CanvasRenderingContext2D {
  readonly attribute HTMLCanvasElement canvas;
  attribute any fillStyle;
  attribute any strokeStyle;
  attribute unrestricted double lineWidth;
  attribute DOMString lineCap;
  attribute DOMString lineJoin;
  attribute unrestricted double miterLimit;
  attribute unrestricted double lineDashOffset;
  undefined setLineDash(sequence<unrestricted double> segments);
  sequence<unrestricted double> getLineDash();
  attribute unrestricted double globalAlpha;
  attribute DOMString globalCompositeOperation;
  attribute boolean imageSmoothingEnabled;
  undefined save();
  undefined reset();
  undefined restore();
  undefined beginPath();
  undefined closePath();
  undefined moveTo(unrestricted double x, unrestricted double y);
  undefined lineTo(unrestricted double x, unrestricted double y);
  undefined rect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined roundRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h, optional any radii);
  undefined quadraticCurveTo(unrestricted double cpx, unrestricted double cpy, unrestricted double x, unrestricted double y);
  undefined bezierCurveTo(unrestricted double cp1x, unrestricted double cp1y, unrestricted double cp2x, unrestricted double cp2y, unrestricted double x, unrestricted double y);
  undefined arc(unrestricted double x, unrestricted double y, unrestricted double radius, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
  undefined arcTo(unrestricted double x1, unrestricted double y1, unrestricted double x2, unrestricted double y2, unrestricted double radius);
  undefined ellipse(unrestricted double x, unrestricted double y, unrestricted double radiusX, unrestricted double radiusY, unrestricted double rotation, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
  undefined fill(optional DOMString fillRule = "nonzero");
  undefined fill(Path2D path, optional DOMString fillRule = "nonzero");
  undefined clip(optional DOMString fillRule = "nonzero");
  undefined clip(Path2D path, optional DOMString fillRule = "nonzero");
  undefined stroke();
  undefined stroke(Path2D path);
  undefined strokeRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  boolean isPointInStroke(unrestricted double x, unrestricted double y);
  boolean isPointInStroke(Path2D path, unrestricted double x, unrestricted double y);
  boolean isPointInPath(unrestricted double x, unrestricted double y, optional DOMString fillRule = "nonzero");
  boolean isPointInPath(Path2D path, unrestricted double x, unrestricted double y, optional DOMString fillRule = "nonzero");
  undefined scale(unrestricted double x, unrestricted double y);
  undefined rotate(unrestricted double angle);
  undefined translate(unrestricted double x, unrestricted double y);
  undefined transform(unrestricted double a, unrestricted double b, unrestricted double c, unrestricted double d, unrestricted double e, unrestricted double f);
  undefined setTransform(optional any transform);
  undefined setTransform(unrestricted double a, unrestricted double b, unrestricted double c, unrestricted double d, unrestricted double e, unrestricted double f);
  undefined resetTransform();
  DOMMatrix getTransform();
  undefined clearRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined fillRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined drawImage(any image, unrestricted double dx, unrestricted double dy);
  undefined drawImage(any image, unrestricted double dx, unrestricted double dy, unrestricted double dw, unrestricted double dh);
  undefined drawImage(any image, unrestricted double sx, unrestricted double sy, unrestricted double sw, unrestricted double sh, unrestricted double dx, unrestricted double dy, unrestricted double dw, unrestricted double dh);
  ImageData createImageData(ImageData image);
  ImageData createImageData(long sw, long sh);
  ImageData getImageData(long sx, long sy, long sw, long sh);
  undefined putImageData(ImageData image, long dx, long dy);
  undefined putImageData(ImageData image, long dx, long dy, long dirtyX, long dirtyY, long dirtyWidth, long dirtyHeight);
};

[Exposed=(Window,Worker), Transferable]
interface OffscreenCanvas : EventTarget {
  constructor([EnforceRange] unsigned long long width, [EnforceRange] unsigned long long height);
  attribute unsigned long long width;
  attribute unsigned long long height;
  any getContext(DOMString contextId, optional any options = null);
  ImageBitmap transferToImageBitmap();
  Promise<Blob> convertToBlob(optional any options = null);
};

[Exposed=(Window,Worker)]
interface OffscreenCanvasRenderingContext2D {
  readonly attribute OffscreenCanvas canvas;
  attribute any fillStyle;
  attribute any strokeStyle;
  attribute unrestricted double lineWidth;
  attribute DOMString lineCap;
  attribute DOMString lineJoin;
  attribute unrestricted double miterLimit;
  attribute unrestricted double lineDashOffset;
  undefined setLineDash(sequence<unrestricted double> segments);
  sequence<unrestricted double> getLineDash();
  attribute unrestricted double globalAlpha;
  attribute DOMString globalCompositeOperation;
  attribute boolean imageSmoothingEnabled;
  undefined save();
  undefined reset();
  undefined restore();
  undefined beginPath();
  undefined closePath();
  undefined moveTo(unrestricted double x, unrestricted double y);
  undefined lineTo(unrestricted double x, unrestricted double y);
  undefined rect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined roundRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h, optional any radii);
  undefined quadraticCurveTo(unrestricted double cpx, unrestricted double cpy, unrestricted double x, unrestricted double y);
  undefined bezierCurveTo(unrestricted double cp1x, unrestricted double cp1y, unrestricted double cp2x, unrestricted double cp2y, unrestricted double x, unrestricted double y);
  undefined arc(unrestricted double x, unrestricted double y, unrestricted double radius, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
  undefined arcTo(unrestricted double x1, unrestricted double y1, unrestricted double x2, unrestricted double y2, unrestricted double radius);
  undefined ellipse(unrestricted double x, unrestricted double y, unrestricted double radiusX, unrestricted double radiusY, unrestricted double rotation, unrestricted double startAngle, unrestricted double endAngle, optional boolean counterclockwise = false);
  undefined fill(optional DOMString fillRule = "nonzero");
  undefined fill(Path2D path, optional DOMString fillRule = "nonzero");
  undefined clip(optional DOMString fillRule = "nonzero");
  undefined clip(Path2D path, optional DOMString fillRule = "nonzero");
  undefined stroke();
  undefined stroke(Path2D path);
  undefined strokeRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  boolean isPointInStroke(unrestricted double x, unrestricted double y);
  boolean isPointInStroke(Path2D path, unrestricted double x, unrestricted double y);
  boolean isPointInPath(unrestricted double x, unrestricted double y, optional DOMString fillRule = "nonzero");
  boolean isPointInPath(Path2D path, unrestricted double x, unrestricted double y, optional DOMString fillRule = "nonzero");
  undefined scale(unrestricted double x, unrestricted double y);
  undefined rotate(unrestricted double angle);
  undefined translate(unrestricted double x, unrestricted double y);
  undefined transform(unrestricted double a, unrestricted double b, unrestricted double c, unrestricted double d, unrestricted double e, unrestricted double f);
  undefined setTransform(optional any transform);
  undefined setTransform(unrestricted double a, unrestricted double b, unrestricted double c, unrestricted double d, unrestricted double e, unrestricted double f);
  undefined resetTransform();
  DOMMatrix getTransform();
  undefined clearRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined fillRect(unrestricted double x, unrestricted double y, unrestricted double w, unrestricted double h);
  undefined drawImage(any image, unrestricted double dx, unrestricted double dy);
  undefined drawImage(any image, unrestricted double dx, unrestricted double dy, unrestricted double dw, unrestricted double dh);
  undefined drawImage(any image, unrestricted double sx, unrestricted double sy, unrestricted double sw, unrestricted double sh, unrestricted double dx, unrestricted double dy, unrestricted double dw, unrestricted double dh);
  ImageData createImageData(ImageData image);
  ImageData createImageData(long sw, long sh);
  ImageData getImageData(long sx, long sy, long sw, long sh);
  undefined putImageData(ImageData image, long dx, long dy);
  undefined putImageData(ImageData image, long dx, long dy, long dirtyX, long dirtyY, long dirtyWidth, long dirtyHeight);
};

[Exposed=Window]
interface HTMLImageElement : HTMLElement {
  [CEReactions] attribute DOMString fetchPriority;
  [CEReactions] attribute DOMString srcset;
  [CEReactions] attribute DOMString sizes;
  [CEReactions] attribute DOMString loading;
  readonly attribute USVString currentSrc;
  readonly attribute boolean complete;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute DOMString src;
  [CEReactions] attribute DOMString crossOrigin;
  [CEReactions] attribute DOMString referrerPolicy;
  [CEReactions] attribute unsigned long width;
  [CEReactions] attribute unsigned long height;
  readonly attribute unsigned long naturalWidth;
  readonly attribute unsigned long naturalHeight;
};

enum SelectionMode { "select", "start", "end", "preserve" };

[Exposed=Window]
interface HTMLInputElement : HTMLElement {
  [CEReactions] attribute DOMString align;
  [CEReactions] attribute DOMString useMap;
  [CEReactions] attribute unsigned long width;
  [CEReactions] attribute unsigned long height;
  [CEReactions] attribute unsigned long size;
  [CEReactions] attribute DOMString accept;
  [CEReactions] attribute DOMString alt;
  [CEReactions] attribute DOMString placeholder;
  [CEReactions] attribute DOMString dirName;
  [CEReactions] attribute DOMString autocomplete;
  [CEReactions] attribute USVString src;
  [CEReactions] attribute boolean switch;
  readonly attribute HTMLDataListElement? list;
  [CEReactions] attribute boolean alpha;
  [CEReactions] attribute DOMString colorSpace;
  undefined showPicker();
  [CEReactions] attribute long minLength;
  [CEReactions] attribute long maxLength;
  [CEReactions] attribute DOMString pattern;
  attribute unrestricted double valueAsNumber;
  attribute object? valueAsDate;
  undefined stepUp(optional long n = 1);
  undefined stepDown(optional long n = 1);
  [CEReactions] attribute DOMString min;
  [CEReactions] attribute DOMString max;
  [CEReactions] attribute DOMString step;
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  boolean reportValidity();
  [CEReactions] attribute boolean multiple;
  attribute FileList? files;
  readonly attribute NodeList? labels;
  readonly attribute HTMLFormElement? form;
  [CEReactions] attribute USVString formAction;
  [CEReactions] attribute DOMString formEnctype;
  [CEReactions] attribute DOMString formMethod;
  [CEReactions] attribute boolean formNoValidate;
  [CEReactions] attribute DOMString formTarget;
  [CEReactions] attribute DOMString value;
  [CEReactions] attribute DOMString defaultValue;
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString name;
  attribute boolean checked;
  [CEReactions] attribute boolean defaultChecked;
  attribute boolean indeterminate;
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute boolean readOnly;
  [CEReactions] attribute boolean required;
  attribute unsigned long? selectionStart;
  attribute unsigned long? selectionEnd;
  attribute DOMString? selectionDirection;
  undefined select();
  undefined setSelectionRange(unsigned long start, unsigned long end, optional DOMString direction = "none");
  undefined setRangeText(DOMString replacement);
  undefined setRangeText(DOMString replacement, unsigned long start, unsigned long end, optional SelectionMode selectionMode = "preserve");
  OpaqueRange createValueRange(unsigned long start, unsigned long end);
};

[Exposed=Window]
interface HTMLDataListElement : HTMLElement {
  [SameObject] readonly attribute HTMLCollection options;
};

[Exposed=Window]
interface HTMLTextAreaElement : HTMLElement {
  readonly attribute DOMString type;
  [CEReactions] attribute unsigned long cols;
  [CEReactions] attribute unsigned long rows;
  [CEReactions] attribute DOMString wrap;
  [CEReactions] attribute DOMString placeholder;
  [CEReactions] attribute DOMString dirName;
  [CEReactions] attribute DOMString autocomplete;
  [CEReactions] attribute long minLength;
  [CEReactions] attribute long maxLength;
  readonly attribute unsigned long textLength;
  boolean reportValidity();
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  readonly attribute NodeList labels;
  readonly attribute HTMLFormElement? form;
  attribute DOMString value;
  [CEReactions] attribute DOMString defaultValue;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute boolean readOnly;
  [CEReactions] attribute boolean required;
  attribute unsigned long? selectionStart;
  attribute unsigned long? selectionEnd;
  attribute DOMString? selectionDirection;
  undefined select();
  undefined setSelectionRange(unsigned long start, unsigned long end, optional DOMString direction = "none");
  undefined setRangeText(DOMString replacement);
  undefined setRangeText(DOMString replacement, unsigned long start, unsigned long end, optional SelectionMode selectionMode = "preserve");
  OpaqueRange createValueRange(unsigned long start, unsigned long end);
};

[Exposed=Window]
interface HTMLSelectElement : HTMLElement {
  [CEReactions] attribute DOMString autocomplete;
  undefined showPicker();
  boolean reportValidity();
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  [CEReactions] attribute unsigned long length;
  getter HTMLOptionElement? item(unsigned long index);
  [CEReactions] setter undefined (unsigned long index, HTMLOptionElement? option);
  HTMLOptionElement? namedItem(DOMString name);
  [CEReactions] undefined remove();
  [CEReactions] undefined remove(long index);
  [CEReactions] attribute unsigned long size;
  readonly attribute DOMString type;
  [SameObject] readonly attribute HTMLCollection selectedOptions;
  [CEReactions] undefined add((HTMLOptionElement or HTMLOptGroupElement) element, optional (HTMLElement or long)? before = null);
  readonly attribute NodeList labels;
  readonly attribute HTMLFormElement? form;
  [SameObject] readonly attribute HTMLOptionsCollection options;
  attribute DOMString value;
  attribute long selectedIndex;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute boolean multiple;
  [CEReactions] attribute boolean required;
};

[Exposed=Window]
interface HTMLSelectedContentElement : HTMLElement {};

dictionary ShowPopoverOptions {
  HTMLElement? source = null;
};
dictionary TogglePopoverOptions : ShowPopoverOptions {
  boolean force;
};

[Exposed=Window]
interface ToggleEvent : Event {
  constructor(DOMString type, optional ToggleEventInit eventInitDict = {});
  readonly attribute DOMString oldState;
  readonly attribute DOMString newState;
  readonly attribute Element? source;
};
dictionary ToggleEventInit : EventInit {
  DOMString oldState = "";
  DOMString newState = "";
  Element? source = null;
};

[Exposed=Window]
interface HTMLOptGroupElement : HTMLElement {
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute DOMString label;
};

[Exposed=Window]
interface ValidityState {
  readonly attribute boolean valueMissing;
  readonly attribute boolean typeMismatch;
  readonly attribute boolean patternMismatch;
  readonly attribute boolean tooLong;
  readonly attribute boolean tooShort;
  readonly attribute boolean rangeUnderflow;
  readonly attribute boolean rangeOverflow;
  readonly attribute boolean stepMismatch;
  readonly attribute boolean badInput;
  readonly attribute boolean customError;
  readonly attribute boolean valid;
};

[Exposed=Window]
interface HTMLOptionElement : HTMLElement {
  readonly attribute HTMLFormElement? form;
  readonly attribute long index;
  [CEReactions] attribute DOMString text;
  [CEReactions] attribute DOMString label;
  [CEReactions] attribute DOMString value;
  attribute boolean selected;
  [CEReactions] attribute boolean defaultSelected;
  [CEReactions] attribute boolean disabled;
};

[Exposed=Window]
interface HTMLButtonElement : HTMLElement {
  boolean reportValidity();
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  readonly attribute NodeList labels;
  readonly attribute HTMLFormElement? form;
  [CEReactions] attribute USVString formAction;
  [CEReactions] attribute DOMString formEnctype;
  [CEReactions] attribute DOMString formMethod;
  [CEReactions] attribute boolean formNoValidate;
  [CEReactions] attribute DOMString formTarget;
  [CEReactions] attribute DOMString value;
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute boolean disabled;
};

[Exposed=Window]
interface HTMLLabelElement : HTMLElement {
  [CEReactions] attribute DOMString htmlFor;
  readonly attribute HTMLElement? control;
  readonly attribute HTMLFormElement? form;
};

[Exposed=Window]
interface HTMLMeterElement : HTMLElement {
  readonly attribute NodeList labels;
};

[Exposed=Window]
interface HTMLProgressElement : HTMLElement {
  readonly attribute NodeList labels;
};

[Exposed=Window]
interface HTMLOutputElement : HTMLElement {
  [SameObject, PutForwards=value] readonly attribute DOMTokenList htmlFor;
  [CEReactions] attribute DOMString name;
  readonly attribute DOMString type;
  [CEReactions] attribute DOMString value;
  [CEReactions] attribute DOMString defaultValue;
  boolean reportValidity();
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  readonly attribute NodeList labels;
  readonly attribute HTMLFormElement? form;
};

[Exposed=Window]
interface HTMLFieldSetElement : HTMLElement {
  boolean reportValidity();
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
  [SameObject] readonly attribute HTMLCollection elements;
  readonly attribute HTMLFormElement? form;
  readonly attribute DOMString type;
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute DOMString name;
};

[Exposed=Window]
interface HTMLObjectElement : HTMLElement {
  readonly attribute Window? contentWindow;
  Document? getSVGDocument();
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute DOMString width;
  [CEReactions] attribute DOMString height;
  readonly attribute Document? contentDocument;
  [CEReactions] attribute USVString data;
  boolean reportValidity();
  readonly attribute HTMLFormElement? form;
  readonly attribute boolean willValidate;
  readonly attribute ValidityState validity;
  readonly attribute DOMString validationMessage;
  boolean checkValidity();
  undefined setCustomValidity(DOMString error);
};

[Exposed=Window]
interface HTMLFormElement : HTMLElement {
  [SameObject, PutForwards=value] readonly attribute DOMTokenList relList;
  [SameObject] readonly attribute HTMLFormControlsCollection elements;
  readonly attribute unsigned long length;
  [CEReactions] attribute DOMString acceptCharset;
  [CEReactions] attribute USVString action;
  [CEReactions] attribute DOMString autocomplete;
  [CEReactions] attribute DOMString enctype;
  [CEReactions] attribute DOMString encoding;
  [CEReactions] attribute DOMString method;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute boolean noValidate;
  [CEReactions] attribute DOMString target;
  [CEReactions] attribute DOMString rel;
  boolean checkValidity();
  boolean reportValidity();
  undefined requestSubmit(optional HTMLElement? submitter = null);
  undefined submit();
  [CEReactions] undefined reset();
};

[Exposed=Window]
interface HTMLAnchorElement : HTMLElement {
  [CEReactions] attribute DOMString download;
  [SameObject, PutForwards=value] readonly attribute DOMTokenList relList;
  [CEReactions] attribute DOMString rel;
  [CEReactions] attribute USVString ping;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute USVString href;
  readonly attribute USVString origin;
  [CEReactions] attribute USVString protocol;
  [CEReactions] attribute USVString username;
  [CEReactions] attribute USVString password;
  [CEReactions] attribute USVString host;
  [CEReactions] attribute USVString hostname;
  [CEReactions] attribute USVString port;
  [CEReactions] attribute USVString pathname;
  [CEReactions] attribute USVString search;
  [CEReactions] attribute USVString hash;
  attribute DOMString target;
  [CEReactions] attribute DOMString referrerPolicy;
};

[Exposed=Window]
interface HTMLAreaElement : HTMLElement {
  [CEReactions] attribute DOMString coords;
  [CEReactions] attribute DOMString shape;
  [CEReactions] attribute DOMString download;
  [SameObject, PutForwards=value] readonly attribute DOMTokenList relList;
  [CEReactions] attribute DOMString rel;
  [CEReactions] attribute USVString ping;
  [CEReactions] attribute USVString href;
  readonly attribute USVString origin;
  [CEReactions] attribute USVString protocol;
  [CEReactions] attribute USVString username;
  [CEReactions] attribute USVString password;
  [CEReactions] attribute USVString host;
  [CEReactions] attribute USVString hostname;
  [CEReactions] attribute USVString port;
  [CEReactions] attribute USVString pathname;
  [CEReactions] attribute USVString search;
  [CEReactions] attribute USVString hash;
  attribute DOMString target;
  [CEReactions] attribute DOMString referrerPolicy;
};

[Exposed=Window]
interface HTMLLinkElement : HTMLElement {
  [SameObject, PutForwards=value] readonly attribute DOMTokenList blocking;
  [SameObject, PutForwards=value] readonly attribute DOMTokenList relList;
  [CEReactions] attribute DOMString integrity;
  [CEReactions] attribute DOMString as;
  [CEReactions] attribute DOMString? crossOrigin;
  [CEReactions] attribute DOMString fetchPriority;
  [CEReactions] attribute boolean disabled;
  [CEReactions] attribute DOMString href;
  [CEReactions] attribute DOMString rel;
  [CEReactions] attribute DOMString media;
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString referrerPolicy;
  readonly attribute CSSStyleSheet? sheet;
};

[Exposed=Window]
interface HTMLStyleElement : HTMLElement {
  [SameObject, PutForwards=value] readonly attribute DOMTokenList blocking;
  attribute boolean disabled;
  [CEReactions] attribute DOMString media;
  [CEReactions] attribute DOMString type;
  readonly attribute CSSStyleSheet? sheet;
};

[Exposed=Window]
interface HTMLIFrameElement : HTMLElement {
  Document? getSVGDocument();
  [CEReactions] attribute DOMString allow;
  [CEReactions] attribute boolean allowFullscreen;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute DOMString src;
  [CEReactions] attribute (TrustedHTML or DOMString) srcdoc;
  attribute DOMString sandbox;
  [CEReactions] attribute DOMString referrerPolicy;
  readonly attribute Document? contentDocument;
  readonly attribute any contentWindow;
  [CEReactions] attribute DOMString width;
  [CEReactions] attribute DOMString height;
};

[Exposed=Window]
interface HTMLScriptElement : HTMLElement {
  [SameObject, PutForwards=value] readonly attribute DOMTokenList blocking;
  [CEReactions] attribute DOMString charset;
  [CEReactions] attribute DOMString event;
  [CEReactions] attribute DOMString htmlFor;
  [CEReactions] attribute DOMString fetchPriority;
  static boolean supports(DOMString type);
  [CEReactions] attribute boolean async;
  [CEReactions] attribute boolean defer;
  [CEReactions] attribute boolean noModule;
  [CEReactions] attribute DOMString? crossOrigin;
  [CEReactions] attribute DOMString integrity;
  [CEReactions] attribute (TrustedScriptURL or USVString) src;
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString referrerPolicy;
  [CEReactions] attribute (TrustedScript or DOMString) text;
  [CEReactions] attribute (TrustedScript or DOMString)? textContent;
  [CEReactions] attribute (TrustedScript or DOMString) innerText;
};

[Exposed=Window]
interface HTMLTableElement : HTMLElement {
  [HTMLConstructor] constructor();
  [SameObject] readonly attribute HTMLCollection rows;
};

[Exposed=Window]
interface HTMLDetailsElement : HTMLElement {
  [HTMLConstructor] constructor();
  [CEReactions] attribute boolean open;
  [CEReactions] attribute DOMString name;
};

[Exposed=Window]
interface HTMLDialogElement : HTMLElement {
  [CEReactions] attribute boolean open;
  attribute DOMString returnValue;
  [CEReactions] undefined show();
  [CEReactions] undefined showModal();
  [CEReactions] undefined close(optional DOMString returnValue);
  [CEReactions] undefined requestClose(optional DOMString returnValue);
};

[Exposed=Window]
interface Document : Node {
  [CEReactions] attribute DOMString title;
  static Document parseHTMLUnsafe((TrustedHTML or DOMString) html, optional ParseHTMLUnsafeOptions options = {});
  static Document parseHTML(DOMString html, optional SetHTMLOptions options = {});
  readonly attribute CustomElementRegistry? customElementRegistry;
  [CEReactions, Unscopable] undefined append((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined prepend((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceChildren((Node or DOMString)... nodes);
  [CEReactions] undefined moveBefore(Node movedNode, Node? referenceNode);

  attribute EventListener? ondragstart;
  attribute EventListener? ondrag;
  attribute EventListener? ondragenter;
  attribute EventListener? ondragleave;
  attribute EventListener? ondragover;
  attribute EventListener? ondrop;
  attribute EventListener? ondragend;
  attribute EventListener? oncopy;
  attribute EventListener? oncut;
  attribute EventListener? onpaste;

  readonly attribute boolean prerendering;
  attribute EventListener? onprerenderingchange;
  readonly attribute DOMString readyState;
  [SameObject] readonly attribute HTMLAllCollection all;
  [SameObject] readonly attribute HTMLCollection images;
  [SameObject] readonly attribute HTMLCollection embeds;
  [SameObject] readonly attribute HTMLCollection plugins;
  [SameObject] readonly attribute HTMLCollection links;
  [SameObject] readonly attribute HTMLCollection forms;
  [SameObject] readonly attribute HTMLCollection scripts;
  attribute ObservableArray<CSSStyleSheet> adoptedStyleSheets;
  constructor();
  readonly attribute DOMImplementation implementation;
  readonly attribute DOMString URL;
  readonly attribute DOMString documentURI;
  readonly attribute DOMString compatMode;
  readonly attribute DOMString referrer;
  readonly attribute DOMString characterSet;
  readonly attribute DOMString charset;
  readonly attribute DOMString inputEncoding;
  readonly attribute DOMString contentType;
  [PutForwards=href] readonly attribute Location? location;
  attribute DOMString cookie;
  attribute DOMString domain;
  readonly attribute any defaultView;
  [SameObject] readonly attribute HTMLCollection children;
  readonly attribute Element? firstElementChild;
  readonly attribute Element? lastElementChild;
  readonly attribute unsigned long childElementCount;
  readonly attribute HTMLElement? documentElement;
  readonly attribute HTMLElement? head;
  [CEReactions] readonly attribute HTMLElement? body;
  readonly attribute Element? scrollingElement;
  readonly attribute Node? doctype;
  readonly attribute Element? activeElement;
  readonly attribute HTMLElement? currentScript;
  [SameObject] readonly attribute StyleSheetList styleSheets;
  [CEReactions] attribute DOMString designMode;
  [CEReactions] Node importNode(Node node, optional (boolean or ImportNodeOptions) options = false);
  [CEReactions] Node adoptNode(Node node);
  Attr createAttribute(DOMString localName);
  Attr createAttributeNS(DOMString? namespace, DOMString qualifiedName);
  [CEReactions] Element createElement(DOMString localName, optional (DOMString or ElementCreationOptions) options = {});
  [CEReactions] Element createElementNS(DOMString? namespace, DOMString qualifiedName, optional (DOMString or ElementCreationOptions) options = {});
  Text createTextNode(DOMString data);
  Comment createComment(DOMString data);
  [NewObject] CDATASection createCDATASection(DOMString data);
  [NewObject] ProcessingInstruction createProcessingInstruction(DOMString target, DOMString data);
  DocumentFragment createDocumentFragment();
  Event createEvent(DOMString interfaceName);
  Document open();
  [CEReactions] undefined write((TrustedHTML or DOMString)... text);
  [CEReactions] undefined writeln((TrustedHTML or DOMString)... text);
  [CEReactions] undefined close();
  Element? getElementById(DOMString id);
  Element? querySelector(DOMString selectors);
  [NewObject] NodeList querySelectorAll(DOMString selectors);
  HTMLCollection getElementsByTagName(DOMString qualifiedName);
  HTMLCollection getElementsByTagNameNS(DOMString? namespace, DOMString localName);
  HTMLCollection getElementsByClassName(DOMString classNames);
  NodeList getElementsByName(DOMString elementName);
  Element? elementFromPoint(double x, double y);
  sequence<Element> elementsFromPoint(double x, double y);
  Range createRange();
  NodeIterator createNodeIterator(Node root, optional unsigned long whatToShow = 4294967295, optional NodeFilter? filter = null);
  TreeWalker createTreeWalker(Node root, optional unsigned long whatToShow = 4294967295, optional NodeFilter? filter = null);
  Selection? getSelection();
  [CEReactions] boolean execCommand(DOMString commandId, optional boolean showUI = false, optional DOMString value = "");
  boolean queryCommandEnabled(DOMString commandId);
  boolean queryCommandIndeterm(DOMString commandId);
  boolean queryCommandState(DOMString commandId);
  boolean queryCommandSupported(DOMString commandId);
  DOMString queryCommandValue(DOMString commandId);
};

[Exposed=Window]
interface XSLTProcessor {
  constructor();
  undefined importStylesheet(Node style);
  DocumentFragment? transformToFragment(Node source, Document output);
  Document? transformToDocument(Node source);
  undefined setParameter(DOMString? namespaceURI, DOMString localName, DOMString value);
  DOMString? getParameter(DOMString? namespaceURI, DOMString localName);
  undefined removeParameter(DOMString? namespaceURI, DOMString localName);
  undefined clearParameters();
  undefined reset();
};

[Exposed=Window]
interface XMLSerializer {
  constructor();
  DOMString serializeToString(Node root);
};

enum SupportedType { "text/html", "text/xml", "application/xml", "application/xhtml+xml", "image/svg+xml" };

[Exposed=Window]
interface DOMParser {
  constructor();
  Document parseFromString((DOMString or TrustedHTML) input, SupportedType type);
};

[Exposed=Window]
interface DOMImplementation {
  boolean hasFeature();
  [NewObject] XMLDocument createDocument(DOMString? namespace, [LegacyNullToEmptyString] DOMString qualifiedName, optional DocumentType? doctype = null);
  [NewObject] Document createHTMLDocument(optional DOMString title);
  [NewObject] DocumentType createDocumentType(DOMString name, DOMString publicId, DOMString systemId);
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
interface OpaqueRange : AbstractRange {
  undefined disconnect();
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
  DOMRectList getClientRects();
  DOMRect getBoundingClientRect();
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
  [CEReactions] undefined deleteContents();
  [CEReactions] DocumentFragment extractContents();
  [CEReactions] DocumentFragment cloneContents();
  [CEReactions] undefined insertNode(Node node);
  [CEReactions] undefined surroundContents(Node newParent);
  Range cloneRange();
  undefined detach();
  boolean isPointInRange(Node node, unsigned long offset);
  short comparePoint(Node node, unsigned long offset);
  boolean intersectsNode(Node node);
  DOMString toString();
  [CEReactions] DocumentFragment createContextualFragment(DOMString fragment);
};

dictionary GetComposedRangesOptions {
  sequence<ShadowRoot> shadowRoots = [];
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
  sequence<StaticRange> getComposedRanges(optional GetComposedRangesOptions options = {});
  Range getRangeAt(unsigned long index);
  undefined addRange(Range range);
  undefined removeRange(Range range);
  undefined removeAllRanges();
  undefined empty();
  undefined collapse(Node? node, optional unsigned long offset = 0);
  undefined setPosition(Node? node, optional unsigned long offset = 0);
  undefined extend(Node node, optional unsigned long offset = 0);
  undefined setBaseAndExtent(Node anchorNode, unsigned long anchorOffset, Node focusNode, unsigned long focusOffset);
  undefined modify(optional DOMString alter, optional DOMString direction, optional DOMString granularity);
  undefined collapseToStart();
  undefined collapseToEnd();
  undefined selectAllChildren(Node node);
  [CEReactions] undefined deleteFromDocument();
  boolean containsNode(Node node, optional boolean allowPartialContainment = false);
  DOMString toString();
};

[Exposed=Window]
callback interface NodeFilter {
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
  readonly attribute NodeFilter? filter;
  Node? nextNode();
  Node? previousNode();
  undefined detach();
};

[Exposed=Window]
interface TreeWalker {
  [SameObject] readonly attribute Node root;
  readonly attribute unsigned long whatToShow;
  readonly attribute NodeFilter? filter;
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
  Promise<sequence<IDBDatabaseInfo>> databases();
};

[Exposed=(Window,Worker)]
interface IDBVersionChangeEvent : Event {
  constructor(DOMString type, optional IDBVersionChangeEventInit eventInitDict = {});
  readonly attribute unsigned long long oldVersion;
  readonly attribute unsigned long long? newVersion;
};

dictionary IDBDatabaseInfo {
  DOMString name;
  unsigned long long version;
};

dictionary IDBVersionChangeEventInit : EventInit {
  unsigned long long oldVersion = 0;
  unsigned long long? newVersion = null;
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
  readonly attribute DOMStringList objectStoreNames;
  attribute EventListener? onabort;
  attribute EventListener? onclose;
  attribute EventListener? onerror;
  attribute EventListener? onversionchange;
  [NewObject] IDBObjectStore createObjectStore(DOMString name, optional IDBObjectStoreParameters options = {});
  undefined deleteObjectStore(DOMString name);
  IDBTransaction transaction((DOMString or sequence<DOMString>) storeNames, optional IDBTransactionMode mode = "readonly", optional IDBTransactionOptions options = {});
  undefined close();
};

enum IDBTransactionMode { "readonly", "readwrite", "versionchange" };
enum IDBTransactionDurability { "default", "strict", "relaxed" };
dictionary IDBObjectStoreParameters {
  (DOMString or sequence<DOMString>)? keyPath = null;
  boolean autoIncrement = false;
};

dictionary IDBTransactionOptions {
  IDBTransactionDurability durability = "default";
};

[Exposed=(Window,Worker)]
interface IDBTransaction : EventTarget {
  readonly attribute IDBDatabase db;
  readonly attribute IDBTransactionMode mode;
  readonly attribute IDBTransactionDurability durability;
  readonly attribute any error;
  readonly attribute DOMStringList objectStoreNames;
  attribute EventListener? onabort;
  attribute EventListener? oncomplete;
  attribute EventListener? onerror;
  IDBObjectStore objectStore(DOMString name);
  undefined abort();
  undefined commit();
};

[Exposed=(Window,Worker)]
interface IDBKeyRange {
  [NewObject] static IDBKeyRange only(any value);
  [NewObject] static IDBKeyRange lowerBound(any lower, optional boolean open = false);
  [NewObject] static IDBKeyRange upperBound(any upper, optional boolean open = false);
  [NewObject] static IDBKeyRange bound(any lower, any upper, optional boolean lowerOpen = false, optional boolean upperOpen = false);
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
  undefined advance([EnforceRange] unsigned long count);
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
  IDBRequest getAllRecords(optional any options);
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
};

[Exposed=(Window,Worker)]
interface IDBObjectStore {
  attribute DOMString name;
  readonly attribute any keyPath;
  readonly attribute DOMStringList indexNames;
  readonly attribute IDBTransaction transaction;
  readonly attribute boolean autoIncrement;
  any add(any value, optional any key);
  any put(any value, optional any key);
  any get(any query);
  any getKey(any query);
  any getAll(optional any query, optional unsigned long count);
  any getAllKeys(optional any query, optional unsigned long count);
  IDBRequest getAllRecords(optional any options);
  any delete(any query);
  any clear();
  any count(optional any query);
  any openCursor(optional any query, optional DOMString direction = "next");
  any openKeyCursor(optional any query, optional DOMString direction = "next");
  [NewObject] IDBIndex createIndex(DOMString name, (DOMString or sequence<DOMString>) keyPath, optional IDBIndexParameters options = {});
  undefined deleteIndex(DOMString name);
  IDBIndex index(DOMString name);
};

dictionary IDBIndexParameters {
  boolean unique = false;
  boolean multiEntry = false;
};

[Exposed=Window]
interface CSSRuleList {
  getter CSSRule? item(unsigned long index);
  readonly attribute unsigned long length;
};

[Exposed=Window]
interface CSSRule {
  attribute DOMString cssText;
  readonly attribute CSSRule? parentRule;
  readonly attribute CSSStyleSheet? parentStyleSheet;
  readonly attribute unsigned short type;
  const unsigned short STYLE_RULE = 1;
  const unsigned short CHARSET_RULE = 2;
  const unsigned short IMPORT_RULE = 3;
  const unsigned short MEDIA_RULE = 4;
  const unsigned short FONT_FACE_RULE = 5;
  const unsigned short PAGE_RULE = 6;
  const unsigned short MARGIN_RULE = 9;
  const unsigned short NAMESPACE_RULE = 10;
};

[Exposed=Window]
interface CSSImportRule : CSSRule {
  readonly attribute USVString href;
  [SameObject, PutForwards=mediaText] readonly attribute MediaList media;
  [SameObject] readonly attribute CSSStyleSheet? styleSheet;
  readonly attribute CSSOMString? layerName;
  readonly attribute CSSOMString? supportsText;
};

[Exposed=Window]
interface CSSLayerStatementRule : CSSRule {
  readonly attribute FrozenArray<CSSOMString> nameList;
};

[Exposed=Window]
interface CSSNamespaceRule : CSSRule {
  readonly attribute DOMString namespaceURI;
  readonly attribute DOMString prefix;
};

[Exposed=Window]
interface CSSGroupingRule : CSSRule {
  [SameObject] readonly attribute CSSRuleList cssRules;
  unsigned long insertRule(DOMString rule, optional unsigned long index = 0);
  undefined deleteRule(unsigned long index);
};

[Exposed=Window]
interface CSSConditionRule : CSSGroupingRule {
  readonly attribute DOMString conditionText;
};

[Exposed=Window]
interface CSSSupportsRule : CSSConditionRule {
  readonly attribute boolean matches;
};

dictionary CSSContainerCondition {
  required DOMString name;
  required DOMString query;
};

[Exposed=Window]
interface CSSContainerRule : CSSConditionRule {
  readonly attribute DOMString containerName;
  readonly attribute DOMString containerQuery;
  readonly attribute FrozenArray<CSSContainerCondition> conditions;
};

[Exposed=Window]
interface CSSLayerBlockRule : CSSGroupingRule {
  readonly attribute DOMString name;
};

[Exposed=Window]
interface CSSScopeRule : CSSGroupingRule {
  readonly attribute DOMString? start;
  readonly attribute DOMString? end;
};

[Exposed=Window]
interface CSSStartingStyleRule : CSSGroupingRule {
};

[Exposed=Window]
interface CSSPageRule : CSSGroupingRule {
  attribute DOMString selectorText;
  [SameObject, PutForwards=cssText] readonly attribute CSSPageDescriptors style;
};

[Exposed=Window]
interface CSSStyleRule : CSSGroupingRule {
  attribute DOMString selectorText;
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleProperties style;
};

[Exposed=Window]
interface CSSNestedDeclarations : CSSRule {
  [SameObject, PutForwards=cssText] readonly attribute CSSStyleProperties style;
};

[Exposed=Window]
interface CSSMediaRule : CSSConditionRule {
  [SameObject, PutForwards=mediaText] readonly attribute MediaList media;
};

[Exposed=Window]
interface MediaList {
  [LegacyNullToEmptyString] attribute DOMString mediaText;
  readonly attribute unsigned long length;
  getter DOMString? item(unsigned long index);
  undefined appendMedium(DOMString medium);
  undefined deleteMedium(DOMString medium);
  DOMString toString();
};

[Exposed=Window]
interface StyleSheet {
  readonly attribute DOMString type;
  readonly attribute USVString? href;
  readonly attribute Node? ownerNode;
  readonly attribute StyleSheet? parentStyleSheet;
  readonly attribute DOMString? title;
  attribute boolean disabled;
  [SameObject, PutForwards=mediaText] readonly attribute MediaList media;
};

[Exposed=Window]
interface CSSStyleSheet : StyleSheet {
  constructor();
  readonly attribute CSSRule? ownerRule;
  [SameObject] readonly attribute CSSRuleList cssRules;
  [SameObject] readonly attribute CSSRuleList rules;
  unsigned long insertRule(DOMString rule, optional unsigned long index = 0);
  undefined deleteRule(unsigned long index);
  long addRule(optional DOMString selector, optional DOMString style, optional unsigned long index);
  undefined removeRule(optional unsigned long index = 0);
  undefined replaceSync(DOMString text);
  Promise<CSSStyleSheet> replace(DOMString text);
};

[Exposed=Window]
interface CSSStyleProperties : CSSStyleDeclaration {
  [CEReactions] attribute DOMString cssFloat;
};

[Exposed=Window]
interface CSSPageDescriptors : CSSStyleDeclaration {
  attribute DOMString margin;
  attribute DOMString marginTop;
  attribute DOMString marginRight;
  attribute DOMString marginBottom;
  attribute DOMString marginLeft;
  attribute DOMString margin-top;
  attribute DOMString margin-right;
  attribute DOMString margin-bottom;
  attribute DOMString margin-left;
  attribute DOMString size;
  attribute DOMString pageOrientation;
  attribute DOMString page-orientation;
  attribute DOMString marks;
  attribute DOMString bleed;
};

[Exposed=Window]
interface CSSStyleDeclaration {
  DOMString item(unsigned long index);
  readonly attribute CSSRule? parentRule;
  attribute DOMString transition;
  attribute DOMString webkitTransition;
  [CEReactions] attribute DOMString cssText;
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
  [CEReactions] undefined setProperty(DOMString property, DOMString value, optional DOMString priority = "");
  [CEReactions] DOMString removeProperty(DOMString property);
};

[Exposed=Window]
interface DocumentFragment : Node {
  [CEReactions, Unscopable] undefined append((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined prepend((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceChildren((Node or DOMString)... nodes);
  [CEReactions] undefined moveBefore(Node movedNode, Node? referenceNode);

  constructor();
  Element? getElementById(DOMString elementId);
  [SameObject] readonly attribute HTMLCollection children;
  readonly attribute unsigned long childElementCount;
  readonly attribute Element? firstElementChild;
  readonly attribute Element? lastElementChild;
  Element? querySelector(DOMString selectors);
  [NewObject] NodeList querySelectorAll(DOMString selectors);
};

[Exposed=Window]
interface ShadowRoot : DocumentFragment {
  WritableStream streamHTML(optional SetHTMLOptions options = {});
  WritableStream streamHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamAppendHTML(optional SetHTMLOptions options = {});
  WritableStream streamAppendHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamPrependHTML(optional SetHTMLOptions options = {});
  WritableStream streamPrependHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  [CEReactions] undefined appendHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined appendHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined prependHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined prependHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  [CEReactions] undefined setHTML(DOMString html, optional SetHTMLOptions options = {});
  [CEReactions] undefined setHTMLUnsafe((TrustedHTML or DOMString) html, optional SetHTMLUnsafeOptions options = {});
  DOMString getHTML(optional GetHTMLOptions options = {});
  readonly attribute boolean serializable;
  readonly attribute CustomElementRegistry? customElementRegistry;
  attribute DOMString? referenceTarget;
  readonly attribute boolean delegatesFocus;
  readonly attribute DOMString slotAssignment;
  readonly attribute Element? activeElement;
  attribute ObservableArray<CSSStyleSheet> adoptedStyleSheets;
  [CEReactions] attribute DOMString innerHTML;
  readonly attribute DOMString mode;
  readonly attribute boolean clonable;
  readonly attribute Element host;
};

[Exposed=Window]
interface CharacterData : Node {
  readonly attribute Element? previousElementSibling;
  readonly attribute Element? nextElementSibling;
  WritableStream streamBeforeHTML(optional SetHTMLOptions options = {});
  WritableStream streamBeforeHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamAfterHTML(optional SetHTMLOptions options = {});
  WritableStream streamAfterHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  WritableStream streamReplaceWithHTML(optional SetHTMLOptions options = {});
  WritableStream streamReplaceWithHTMLUnsafe(optional (TrustedParserOptions or SetHTMLUnsafeOptions) options = {});
  [CEReactions, Unscopable] undefined before((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined after((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined replaceWith((Node or DOMString)... nodes);
  [CEReactions, Unscopable] undefined remove();

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
  readonly attribute HTMLSlotElement? assignedSlot;
  constructor(optional DOMString data = "");
  readonly attribute DOMString wholeText;
  Text splitText(unsigned long offset);
};

[Exposed=Window]
interface CDATASection : Text {};

[Exposed=Window]
interface ProcessingInstruction : CharacterData {
    readonly attribute CSSStyleSheet? sheet;
  readonly attribute DOMString target;
};

[Exposed=Window]
interface Comment : CharacterData {
  constructor(optional DOMString data = "");
};

[Exposed=(Window,Worker)]
interface Blob {
  constructor(optional sequence<any> blobParts = [], optional any options = null);
  readonly attribute unsigned long long size;
  readonly attribute DOMString type;
  Blob slice(optional [Clamp] long long start = 0, optional [Clamp] long long end, optional DOMString contentType = "");
  Promise<ArrayBuffer> arrayBuffer();
  Promise<USVString> text();
  Promise<Uint8Array> bytes();
  ReadableStream textStream();
  ReadableStream stream();
};

[Exposed=(Window,Worker)]
interface FileList {
  readonly attribute unsigned long length;
  getter File? item(unsigned long index);
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
  const unsigned short EMPTY = 0;
  const unsigned short LOADING = 1;
  const unsigned short DONE = 2;
  readonly attribute any result;
  readonly attribute DOMException? error;
  readonly attribute unsigned short readyState;
  attribute EventListener? onloadstart;
  attribute EventListener? onprogress;
  attribute EventListener? onload;
  attribute EventListener? onerror;
  attribute EventListener? onabort;
  attribute EventListener? onloadend;
  undefined readAsArrayBuffer(Blob blob);
  undefined readAsBinaryString(Blob blob);
  undefined readAsText(Blob blob, optional DOMString encoding);
  undefined readAsDataURL(Blob blob);
  undefined abort();
};

[Exposed=(Window,Worker)]
interface FormData {
  constructor(optional HTMLFormElement form, optional HTMLElement? submitter = null);
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
  sequence<ByteString> getSetCookie();
  boolean has(DOMString name);
  undefined set(DOMString name, DOMString value);
  iterable<DOMString, DOMString>;
};

[Exposed=(Window,Worker)]
interface AbortSignal : EventTarget {
  static AbortSignal abort(optional any reason);
  static AbortSignal timeout([EnforceRange] unsigned long long milliseconds);
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
interface CountQueuingStrategy {
  constructor(QueuingStrategyInit init);
  readonly attribute unrestricted double highWaterMark;
  readonly attribute any size;
};

[Exposed=(Window,Worker)]
interface ByteLengthQueuingStrategy {
  constructor(QueuingStrategyInit init);
  readonly attribute unrestricted double highWaterMark;
  readonly attribute any size;
};

dictionary QueuingStrategyInit {
  required unrestricted double highWaterMark;
};

[Exposed=(Window,Worker)]
interface ReadableStream {
  constructor(optional any underlyingSource = null, optional any strategy = null);
  readonly attribute boolean locked;
  any getReader(optional any options = null);
  sequence<ReadableStream> tee();
  Promise<undefined> cancel(optional any reason = null);
  Promise<undefined> pipeTo(WritableStream destination, optional any options = null);
  ReadableStream pipeThrough(any transform, optional any options = null);
};

[Exposed=(Window,Worker)]
interface ReadableStreamDefaultReader {
  constructor(ReadableStream stream);
  readonly attribute Promise<undefined> closed;
  Promise<any> read();
  Promise<undefined> cancel(optional any reason = null);
  undefined releaseLock();
};

[Exposed=(Window,Worker)]
interface ReadableStreamBYOBReader {
  constructor(ReadableStream stream);
  readonly attribute Promise<undefined> closed;
  Promise<any> read(ArrayBufferView view, optional any options = null);
  Promise<undefined> cancel(optional any reason = null);
  undefined releaseLock();
};
[Exposed=(Window,Worker)]
interface ReadableByteStreamController {
  readonly attribute unrestricted double? desiredSize;
  readonly attribute ReadableStreamBYOBRequest? byobRequest;
  undefined close();
  undefined enqueue(ArrayBufferView chunk);
  undefined error(optional any error = null);
};
[Exposed=(Window,Worker)]
interface ReadableStreamBYOBRequest {
  readonly attribute ArrayBufferView? view;
  undefined respond([EnforceRange] unsigned long long bytesWritten);
  undefined respondWithNewView(ArrayBufferView view);
};

[Exposed=(Window,Worker)]
interface ReadableStreamDefaultController {
  readonly attribute unrestricted double? desiredSize;
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
  readonly attribute unrestricted double? desiredSize;
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
interface TransformStreamDefaultController {
  readonly attribute unrestricted double? desiredSize;
  undefined enqueue(optional any chunk);
  undefined error(optional any reason);
  undefined terminate();
};

[Exposed=(Window,Worker)]
interface TransformStream {
  constructor(optional any transformer = null, optional any writableStrategy = null, optional any readableStrategy = null);
  readonly attribute ReadableStream readable;
  readonly attribute WritableStream writable;
};

[Exposed=(Window,Worker)]
interface Request {
  readonly attribute boolean keepalive;
  readonly attribute DOMString cache;
  constructor(any input, optional any init = null);
  readonly attribute boolean isReloadNavigation;
  readonly attribute boolean isHistoryNavigation;
  readonly attribute DOMString url;
  readonly attribute DOMString method;
  readonly attribute Headers headers;
  readonly attribute AbortSignal signal;
  readonly attribute DOMString integrity;
  readonly attribute DOMString credentials;
  readonly attribute DOMString mode;
  readonly attribute DOMString redirect;
  readonly attribute DOMString destination;
  readonly attribute DOMString referrer;
  readonly attribute DOMString referrerPolicy;
  readonly attribute ReadableStream? body;
  readonly attribute boolean bodyUsed;
  Request clone();
  Promise<ArrayBuffer> arrayBuffer();
  Promise<Uint8Array> bytes();
  Promise<DOMString> text();
  Promise<any> json();
  Promise<Blob> blob();
  Promise<FormData> formData();
};

[Exposed=(Window,Worker)]
interface Response {
  readonly attribute DOMString type;
  constructor(optional any body = null, optional any init = null);
  readonly attribute unsigned short status;
  readonly attribute ByteString statusText;
  readonly attribute boolean ok;
  readonly attribute DOMString url;
  readonly attribute boolean redirected;
  readonly attribute Headers headers;
  readonly attribute ReadableStream? body;
  readonly attribute boolean bodyUsed;
  Response clone();
  Promise<DOMString> text();
  Promise<any> json();
  Promise<ArrayBuffer> arrayBuffer();
  Promise<Uint8Array> bytes();
  Promise<Blob> blob();
  Promise<FormData> formData();
};

[Exposed=(Window,Worker)]
interface XMLHttpRequestEventTarget : EventTarget {
  attribute EventListener? onloadstart;
  attribute EventListener? onprogress;
  attribute EventListener? onabort;
  attribute EventListener? onerror;
  attribute EventListener? ontimeout;
  attribute EventListener? onload;
  attribute EventListener? onloadend;
};

[Exposed=(Window,Worker)]
interface XMLHttpRequestUpload : XMLHttpRequestEventTarget {
};

[Exposed=(Window,Worker)]
interface XMLHttpRequest : XMLHttpRequestEventTarget {
  constructor();
  const unsigned short UNSENT = 0;
  const unsigned short OPENED = 1;
  const unsigned short HEADERS_RECEIVED = 2;
  const unsigned short LOADING = 3;
  const unsigned short DONE = 4;
  readonly attribute XMLHttpRequestUpload upload;
  readonly attribute unsigned short readyState;
  attribute unsigned long timeout;
  attribute boolean withCredentials;
  attribute DOMString responseType;
  readonly attribute unsigned short status;
  readonly attribute ByteString statusText;
  readonly attribute DOMString responseURL;
  readonly attribute DOMString responseText;
  readonly attribute any response;
  attribute EventListener? onreadystatechange;
  undefined open(DOMString method, DOMString url, optional boolean async = true);
  undefined setRequestHeader(DOMString name, DOMString value);
  DOMString? getResponseHeader(DOMString name);
  DOMString getAllResponseHeaders();
  undefined send(optional any body = null);
  undefined abort();
};

[Exposed=(Window,Worker)]
interface WebSocket : EventTarget {
  constructor(USVString url, optional (DOMString or sequence<DOMString>) protocols = []);
  const unsigned short CONNECTING = 0;
  const unsigned short OPEN = 1;
  const unsigned short CLOSING = 2;
  const unsigned short CLOSED = 3;
  readonly attribute USVString url;
  readonly attribute unsigned short readyState;
  readonly attribute unsigned long long bufferedAmount;
  readonly attribute DOMString protocol;
  readonly attribute DOMString extensions;
  attribute DOMString binaryType;
  attribute EventListener? onopen;
  attribute EventListener? onmessage;
  attribute EventListener? onerror;
  attribute EventListener? onclose;
  undefined send((BufferSource or Blob or USVString) data);
  undefined close(optional [Clamp] unsigned short code, optional USVString reason);
};

dictionary WorkerOptions {
  DOMString type = "classic";
  DOMString name = "";
  DOMString credentials = "same-origin";
};

[Exposed=(Window,Worker)]
interface Worker : EventTarget {
  constructor(DOMString scriptURL, optional WorkerOptions options);
  undefined terminate();
  undefined postMessage(any message, optional any transferOrOptions);
  attribute EventHandler onmessage;
  attribute EventHandler onmessageerror;
  attribute EventHandler onerror;
};

[Exposed=(Window,Worker)]
interface SharedWorker : EventTarget {
  constructor(DOMString scriptURL, optional any optionsOrName);
  readonly attribute MessagePort port;
  attribute EventHandler onerror;
};

[Exposed=Worker]
interface ExtendableEvent : Event {
  constructor(DOMString type, optional EventInit eventInitDict = {});
  undefined waitUntil(Promise<any> promise);
};

dictionary FetchEventInit : EventInit {
  required Request request;
  DOMString clientId = "";
  DOMString resultingClientId = "";
  DOMString replacesClientId = "";
};

[Exposed=Worker]
interface FetchEvent : ExtendableEvent {
  constructor(DOMString type, FetchEventInit eventInitDict);
  readonly attribute Request request;
  readonly attribute DOMString clientId;
  readonly attribute DOMString resultingClientId;
  readonly attribute DOMString replacesClientId;
  readonly attribute Promise<any> preloadResponse;
  readonly attribute Promise<undefined> handled;
  undefined respondWith(Promise<Response> response);
};

dictionary ExtendableMessageEventInit : EventInit {
  any data = null;
  DOMString origin = "";
  DOMString lastEventId = "";
  any source = null;
  sequence<MessagePort> ports = [];
};

[Exposed=Worker]
interface ExtendableMessageEvent : ExtendableEvent {
  constructor(DOMString type, optional ExtendableMessageEventInit eventInitDict = {});
  readonly attribute any data;
  readonly attribute DOMString origin;
  readonly attribute DOMString lastEventId;
  readonly attribute any source;
  readonly attribute FrozenArray<MessagePort> ports;
};

[Exposed=(Window,Worker)]
interface TrustedHTML {
  DOMString toString();
  DOMString toJSON();
};

[Exposed=(Window,Worker)]
interface TrustedScript {
  DOMString toString();
  DOMString toJSON();
};

[Exposed=(Window,Worker)]
interface TrustedScriptURL {
  DOMString toString();
  USVString toJSON();
};

[Exposed=(Window,Worker)]
interface TrustedParserOptions {
  readonly attribute boolean runScripts;
};

dictionary TrustedTypePolicyOptions {
  CreateParserOptionsCallback createParserOptions;
  CreateHTMLCallback createHTML;
  CreateScriptCallback createScript;
  CreateScriptURLCallback createScriptURL;
};

callback CreateParserOptionsCallback = SetHTMLUnsafeOptions? (SetHTMLUnsafeOptions input, any... arguments);
callback CreateHTMLCallback = DOMString? (DOMString input, any... arguments);
callback CreateScriptCallback = DOMString? (DOMString input, any... arguments);
callback CreateScriptURLCallback = USVString? (DOMString input, any... arguments);

[Exposed=(Window,Worker)]
interface TrustedTypePolicy {
  TrustedParserOptions createParserOptions(optional SetHTMLUnsafeOptions input = {}, any... arguments);
  readonly attribute DOMString name;
  TrustedHTML createHTML(DOMString input, any... arguments);
  TrustedScript createScript(DOMString input, any... arguments);
  TrustedScriptURL createScriptURL(DOMString input, any... arguments);
};

[Exposed=(Window,Worker)]
interface TrustedTypePolicyFactory {
  readonly attribute TrustedHTML emptyHTML;
  readonly attribute TrustedScript emptyScript;
  readonly attribute TrustedTypePolicy? defaultPolicy;
  TrustedTypePolicy createPolicy(DOMString policyName, optional TrustedTypePolicyOptions? policyOptions = null);
  boolean isHTML(any value);
  boolean isScript(any value);
  boolean isScriptURL(any value);
  DOMString? getAttributeType(DOMString tagName, DOMString attribute, optional DOMString? elementNamespace = null, optional DOMString? attributeNamespace = null);
  DOMString? getPropertyType(DOMString tagName, DOMString property, optional DOMString? elementNamespace = null);
};

dictionary RequestInit {
  boolean keepalive = false;
  DOMString method = "GET";
  DOMString? body = null;
  DOMString referrer = "client";
  DOMString referrerPolicy = "";
};

callback EventListener = undefined (Event event);

[Exposed=(Window,Worker)]
interface IDBRecord {
  readonly attribute any key;
  readonly attribute any primaryKey;
  readonly attribute any value;
};

[Exposed=Window]
interface CSSFontFaceDescriptors : CSSStyleDeclaration {
  [LegacyNullToEmptyString] attribute DOMString src;
  [LegacyNullToEmptyString] attribute DOMString fontFamily;
  [LegacyNullToEmptyString] attribute DOMString font-family;
  [LegacyNullToEmptyString] attribute DOMString fontStyle;
  [LegacyNullToEmptyString] attribute DOMString font-style;
  [LegacyNullToEmptyString] attribute DOMString fontWeight;
  [LegacyNullToEmptyString] attribute DOMString font-weight;
  [LegacyNullToEmptyString] attribute DOMString fontStretch;
  [LegacyNullToEmptyString] attribute DOMString font-stretch;
  [LegacyNullToEmptyString] attribute DOMString fontWidth;
  [LegacyNullToEmptyString] attribute DOMString font-width;
  [LegacyNullToEmptyString] attribute DOMString fontSize;
  [LegacyNullToEmptyString] attribute DOMString font-size;
  [LegacyNullToEmptyString] attribute DOMString sizeAdjust;
  [LegacyNullToEmptyString] attribute DOMString size-adjust;
  [LegacyNullToEmptyString] attribute DOMString unicodeRange;
  [LegacyNullToEmptyString] attribute DOMString unicode-range;
  [LegacyNullToEmptyString] attribute DOMString fontFeatureSettings;
  [LegacyNullToEmptyString] attribute DOMString font-feature-settings;
  [LegacyNullToEmptyString] attribute DOMString fontVariationSettings;
  [LegacyNullToEmptyString] attribute DOMString font-variation-settings;
  [LegacyNullToEmptyString] attribute DOMString fontNamedInstance;
  [LegacyNullToEmptyString] attribute DOMString font-named-instance;
  [LegacyNullToEmptyString] attribute DOMString fontDisplay;
  [LegacyNullToEmptyString] attribute DOMString font-display;
  [LegacyNullToEmptyString] attribute DOMString fontLanguageOverride;
  [LegacyNullToEmptyString] attribute DOMString font-language-override;
  [LegacyNullToEmptyString] attribute DOMString ascentOverride;
  [LegacyNullToEmptyString] attribute DOMString ascent-override;
  [LegacyNullToEmptyString] attribute DOMString descentOverride;
  [LegacyNullToEmptyString] attribute DOMString descent-override;
  [LegacyNullToEmptyString] attribute DOMString lineGapOverride;
  [LegacyNullToEmptyString] attribute DOMString line-gap-override;
  [LegacyNullToEmptyString] attribute DOMString superscriptPositionOverride;
  [LegacyNullToEmptyString] attribute DOMString superscript-position-override;
  [LegacyNullToEmptyString] attribute DOMString subscriptPositionOverride;
  [LegacyNullToEmptyString] attribute DOMString subscript-position-override;
  [LegacyNullToEmptyString] attribute DOMString superscriptSizeOverride;
  [LegacyNullToEmptyString] attribute DOMString superscript-size-override;
  [LegacyNullToEmptyString] attribute DOMString subscriptSizeOverride;
  [LegacyNullToEmptyString] attribute DOMString subscript-size-override;
};

[Exposed=Window]
interface CSSFontFaceRule : CSSRule {
  [SameObject, PutForwards=cssText] readonly attribute CSSFontFaceDescriptors style;
};

partial interface XMLHttpRequest {
  undefined overrideMimeType(DOMString mime);
  [Exposed=Window] readonly attribute Document? responseXML;
};

[Exposed=Window]
interface StyleSheetList {
  getter CSSStyleSheet? item(unsigned long index);
  readonly attribute unsigned long length;
};

// Native drag/clipboard store. Regenerate bindings only at the global generation gate.
[Exposed=Window]
interface DataTransfer {
  constructor();
  attribute DOMString dropEffect;
  attribute DOMString effectAllowed;
  [SameObject] readonly attribute DataTransferItemList items;
  readonly attribute FrozenArray<DOMString> types;
  readonly attribute FileList files;
  undefined clearData(optional DOMString format);
  DOMString getData(DOMString format);
  undefined setData(DOMString format, DOMString data);
  undefined setDragImage(Element image, long x, long y);
};
[Exposed=Window]
interface DataTransferItemList {
  readonly attribute unsigned long length;
  getter DataTransferItem (unsigned long index);
  DataTransferItem? add(DOMString data, DOMString type);
  DataTransferItem? add(File data);
  undefined remove(unsigned long index);
  undefined clear();
};
[Exposed=Window]
interface DataTransferItem {
  readonly attribute DOMString kind;
  readonly attribute DOMString type;
  undefined getAsString(FunctionStringCallback? callback);
  File? getAsFile();
};
dictionary DragEventInit : MouseEventInit { DataTransfer? dataTransfer = null; };
[Exposed=Window]
interface DragEvent : MouseEvent {
  constructor(DOMString type, optional DragEventInit eventInitDict);
  readonly attribute DataTransfer? dataTransfer;
};
dictionary ClipboardEventInit : EventInit { DataTransfer? clipboardData = null; };
[Exposed=Window]
interface ClipboardEvent : Event {
  constructor(DOMString type, optional ClipboardEventInit eventInitDict);
  readonly attribute DataTransfer? clipboardData;
};

callback FunctionStringCallback = undefined (DOMString data);

dictionary ValidityStateFlags {
  boolean valueMissing = false;
  boolean typeMismatch = false;
  boolean patternMismatch = false;
  boolean tooLong = false;
  boolean tooShort = false;
  boolean rangeUnderflow = false;
  boolean rangeOverflow = false;
  boolean stepMismatch = false;
  boolean badInput = false;
  boolean customError = false;
};

partial interface Element {
  [CEReactions] attribute DOMString? role;
  [CEReactions] attribute DOMString? ariaAtomic;
  [CEReactions] attribute DOMString? ariaAutoComplete;
  [CEReactions] attribute DOMString? ariaBrailleLabel;
  [CEReactions] attribute DOMString? ariaBrailleRoleDescription;
  [CEReactions] attribute DOMString? ariaBusy;
  [CEReactions] attribute DOMString? ariaChecked;
  [CEReactions] attribute DOMString? ariaColCount;
  [CEReactions] attribute DOMString? ariaColIndex;
  [CEReactions] attribute DOMString? ariaColIndexText;
  [CEReactions] attribute DOMString? ariaColSpan;
  [CEReactions] attribute DOMString? ariaCurrent;
  [CEReactions] attribute DOMString? ariaDescription;
  [CEReactions] attribute DOMString? ariaDisabled;
  [CEReactions] attribute DOMString? ariaExpanded;
  [CEReactions] attribute DOMString? ariaHasPopup;
  [CEReactions] attribute DOMString? ariaHidden;
  [CEReactions] attribute DOMString? ariaInvalid;
  [CEReactions] attribute DOMString? ariaKeyShortcuts;
  [CEReactions] attribute DOMString? ariaLabel;
  [CEReactions] attribute DOMString? ariaLevel;
  [CEReactions] attribute DOMString? ariaLive;
  [CEReactions] attribute DOMString? ariaModal;
  [CEReactions] attribute DOMString? ariaMultiLine;
  [CEReactions] attribute DOMString? ariaMultiSelectable;
  [CEReactions] attribute DOMString? ariaOrientation;
  [CEReactions] attribute DOMString? ariaPlaceholder;
  [CEReactions] attribute DOMString? ariaPosInSet;
  [CEReactions] attribute DOMString? ariaPressed;
  [CEReactions] attribute DOMString? ariaReadOnly;
  [CEReactions] attribute DOMString? ariaRelevant;
  [CEReactions] attribute DOMString? ariaRequired;
  [CEReactions] attribute DOMString? ariaRoleDescription;
  [CEReactions] attribute DOMString? ariaRowCount;
  [CEReactions] attribute DOMString? ariaRowIndex;
  [CEReactions] attribute DOMString? ariaRowIndexText;
  [CEReactions] attribute DOMString? ariaRowSpan;
  [CEReactions] attribute DOMString? ariaSelected;
  [CEReactions] attribute DOMString? ariaSetSize;
  [CEReactions] attribute DOMString? ariaSort;
  [CEReactions] attribute DOMString? ariaValueMax;
  [CEReactions] attribute DOMString? ariaValueMin;
  [CEReactions] attribute DOMString? ariaValueNow;
  [CEReactions] attribute DOMString? ariaValueText;
};

partial interface ElementInternals {
  attribute DOMString? role;
  attribute DOMString? ariaAtomic;
  attribute DOMString? ariaAutoComplete;
  attribute DOMString? ariaBrailleLabel;
  attribute DOMString? ariaBrailleRoleDescription;
  attribute DOMString? ariaBusy;
  attribute DOMString? ariaChecked;
  attribute DOMString? ariaColCount;
  attribute DOMString? ariaColIndex;
  attribute DOMString? ariaColIndexText;
  attribute DOMString? ariaColSpan;
  attribute DOMString? ariaCurrent;
  attribute DOMString? ariaDescription;
  attribute DOMString? ariaDisabled;
  attribute DOMString? ariaExpanded;
  attribute DOMString? ariaHasPopup;
  attribute DOMString? ariaHidden;
  attribute DOMString? ariaInvalid;
  attribute DOMString? ariaKeyShortcuts;
  attribute DOMString? ariaLabel;
  attribute DOMString? ariaLevel;
  attribute DOMString? ariaLive;
  attribute DOMString? ariaModal;
  attribute DOMString? ariaMultiLine;
  attribute DOMString? ariaMultiSelectable;
  attribute DOMString? ariaOrientation;
  attribute DOMString? ariaPlaceholder;
  attribute DOMString? ariaPosInSet;
  attribute DOMString? ariaPressed;
  attribute DOMString? ariaReadOnly;
  attribute DOMString? ariaRelevant;
  attribute DOMString? ariaRequired;
  attribute DOMString? ariaRoleDescription;
  attribute DOMString? ariaRowCount;
  attribute DOMString? ariaRowIndex;
  attribute DOMString? ariaRowIndexText;
  attribute DOMString? ariaRowSpan;
  attribute DOMString? ariaSelected;
  attribute DOMString? ariaSetSize;
  attribute DOMString? ariaSort;
  attribute DOMString? ariaValueMax;
  attribute DOMString? ariaValueMin;
  attribute DOMString? ariaValueNow;
  attribute DOMString? ariaValueText;
};

partial interface Element {
  attribute FrozenArray<Element>? ariaActionsElements;
  attribute Element? ariaActiveDescendantElement;
  attribute FrozenArray<Element>? ariaControlsElements;
  attribute FrozenArray<Element>? ariaDescribedByElements;
  attribute FrozenArray<Element>? ariaDetailsElements;
  attribute FrozenArray<Element>? ariaErrorMessageElements;
  attribute FrozenArray<Element>? ariaFlowToElements;
  attribute FrozenArray<Element>? ariaLabelledByElements;
  attribute FrozenArray<Element>? ariaOwnsElements;
};
partial interface ElementInternals {
  attribute FrozenArray<Element>? ariaActionsElements;
  attribute Element? ariaActiveDescendantElement;
  attribute FrozenArray<Element>? ariaControlsElements;
  attribute FrozenArray<Element>? ariaDescribedByElements;
  attribute FrozenArray<Element>? ariaDetailsElements;
  attribute FrozenArray<Element>? ariaErrorMessageElements;
  attribute FrozenArray<Element>? ariaFlowToElements;
  attribute FrozenArray<Element>? ariaLabelledByElements;
  attribute FrozenArray<Element>? ariaOwnsElements;
};

[Exposed=Window, SecureContext]
interface Clipboard : EventTarget {
  Promise<sequence<ClipboardItem>> read();
  Promise<DOMString> readText();
  Promise<undefined> write(sequence<ClipboardItem> data);
  Promise<undefined> writeText(DOMString data);
};
[Exposed=Window, SecureContext]
interface ClipboardItem {
  constructor(record<DOMString, Promise<(DOMString or Blob)>> items, optional ClipboardItemOptions options = {});
  readonly attribute FrozenArray<DOMString> types;
  readonly attribute PresentationStyle presentationStyle;
  Promise<Blob> getType(DOMString type);
  static boolean supports(DOMString type);
};
enum PresentationStyle { "unspecified", "inline", "attachment" };
dictionary ClipboardItemOptions { PresentationStyle presentationStyle = "unspecified"; };

// Native service-worker registry wrappers. Generated bindings are refreshed only
// after the complete plan's source-generation gate.
enum ServiceWorkerState { "parsed", "installing", "installed", "activating", "activated", "redundant" };
enum ServiceWorkerUpdateViaCache { "imports", "all", "none" };
dictionary RegistrationOptions {
  USVString scope;
  DOMString type = "classic";
  ServiceWorkerUpdateViaCache updateViaCache = "imports";
};
[Exposed=(Window,Worker), SecureContext]
interface ServiceWorkerContainer : EventTarget {
  readonly attribute ServiceWorker? controller;
  readonly attribute Promise<ServiceWorkerRegistration> ready;
  Promise<ServiceWorkerRegistration> register(USVString scriptURL, optional RegistrationOptions options = {});
  Promise<any> getRegistration(optional USVString clientURL = "");
  Promise<sequence<ServiceWorkerRegistration>> getRegistrations();
  undefined startMessages();
  attribute EventHandler oncontrollerchange;
  attribute EventHandler onmessage;
  attribute EventHandler onmessageerror;
};
[Exposed=(Window,Worker), SecureContext]
interface ServiceWorkerRegistration : EventTarget {
  [SameObject] readonly attribute NavigationPreloadManager navigationPreload;
  readonly attribute ServiceWorker? installing;
  readonly attribute ServiceWorker? waiting;
  readonly attribute ServiceWorker? active;
  readonly attribute USVString scope;
  readonly attribute ServiceWorkerUpdateViaCache updateViaCache;
  Promise<ServiceWorkerRegistration> update();
  Promise<boolean> unregister();
  attribute EventHandler onupdatefound;
};
[Exposed=(Window,Worker), SecureContext]
interface ServiceWorker : EventTarget {
  readonly attribute USVString scriptURL;
  readonly attribute ServiceWorkerState state;
  undefined postMessage(any message, optional any transferOrOptions);
  attribute EventHandler onstatechange;
};

// Native service-agent client capabilities. Public attributes never select the
// client identity used by postMessage or parent registry lookups.
enum ClientType { "window", "worker", "sharedworker", "all" };
enum FrameType { "auxiliary", "top-level", "nested", "none" };
enum DocumentVisibilityState { "visible", "hidden" };
dictionary ClientQueryOptions {
  boolean includeUncontrolled = false;
  ClientType type = "window";
};
[Exposed=ServiceWorker]
interface Client {
  readonly attribute USVString url;
  readonly attribute FrameType frameType;
  readonly attribute DOMString id;
  readonly attribute ClientType type;
  undefined postMessage(any message, optional any transferOrOptions);
};
[Exposed=ServiceWorker]
interface WindowClient : Client {
  readonly attribute DocumentVisibilityState visibilityState;
  readonly attribute boolean focused;
  Promise<WindowClient> focus();
  Promise<WindowClient?> navigate(USVString url);
};
[Exposed=ServiceWorker]
interface Clients {
  Promise<any> get(DOMString id);
  Promise<sequence<Client>> matchAll(optional ClientQueryOptions options = {});
  Promise<undefined> claim();
  Promise<WindowClient?> openWindow(USVString url);
};

// Navigator is a nonconstructible Window interface. Native installers bind
// the realm-owned object and its secure members to this generated prototype.
[Exposed=Window]
interface Navigator {
  [SecureContext] readonly attribute ServiceWorkerContainer serviceWorker;
};

[Exposed=Worker]
interface WorkerNavigator {
  [SecureContext] readonly attribute ServiceWorkerContainer serviceWorker;
};

// Native CacheStorage uses profile-owned service records and native bodies.
typedef (Request or USVString) RequestInfo;
[Exposed=(Window,Worker), SecureContext]
interface CacheStorage {
  Promise<any> match(RequestInfo request, optional MultiCacheQueryOptions options = {});
  Promise<boolean> has(DOMString cacheName);
  Promise<Cache> open(DOMString cacheName);
  Promise<boolean> delete(DOMString cacheName);
  Promise<sequence<DOMString>> keys();
};
[Exposed=(Window,Worker), SecureContext]
interface Cache {
  Promise<any> match(RequestInfo request, optional CacheQueryOptions options = {});
  Promise<sequence<Response>> matchAll(optional RequestInfo request, optional CacheQueryOptions options = {});
  Promise<undefined> add(RequestInfo request);
  Promise<undefined> addAll(sequence<RequestInfo> requests);
  Promise<undefined> put(RequestInfo request, Response response);
  Promise<boolean> delete(RequestInfo request, optional CacheQueryOptions options = {});
  Promise<sequence<Request>> keys(optional RequestInfo request, optional CacheQueryOptions options = {});
};
dictionary CacheQueryOptions {
  boolean ignoreSearch = false;
  boolean ignoreMethod = false;
  boolean ignoreVary = false;
};
dictionary MultiCacheQueryOptions : CacheQueryOptions { DOMString cacheName; };
partial interface Window { [SameObject, SecureContext] readonly attribute CacheStorage caches; };
partial interface WorkerGlobalScope { [SameObject, SecureContext] readonly attribute CacheStorage caches; };

// Native XPath 1.0. Resolver callbacks are invoked without native DOM borrows.
callback interface XPathNSResolver {
  DOMString? lookupNamespaceURI(DOMString prefix);
};
[Exposed=Window]
interface XPathEvaluator {
  constructor();
  XPathExpression createExpression(DOMString expression, optional XPathNSResolver? resolver = null);
  Node createNSResolver(Node nodeResolver);
  XPathResult evaluate(DOMString expression, Node contextNode, optional XPathNSResolver? resolver = null, optional unsigned short type = 0, optional XPathResult? result = null);
};
[Exposed=Window]
interface XPathExpression {
  XPathResult evaluate(Node contextNode, optional unsigned short type = 0, optional XPathResult? result = null);
};
[Exposed=Window]
interface XPathResult {
  const unsigned short ANY_TYPE = 0;
  const unsigned short NUMBER_TYPE = 1;
  const unsigned short STRING_TYPE = 2;
  const unsigned short BOOLEAN_TYPE = 3;
  const unsigned short UNORDERED_NODE_ITERATOR_TYPE = 4;
  const unsigned short ORDERED_NODE_ITERATOR_TYPE = 5;
  const unsigned short UNORDERED_NODE_SNAPSHOT_TYPE = 6;
  const unsigned short ORDERED_NODE_SNAPSHOT_TYPE = 7;
  const unsigned short ANY_UNORDERED_NODE_TYPE = 8;
  const unsigned short FIRST_ORDERED_NODE_TYPE = 9;
  readonly attribute unsigned short resultType;
  readonly attribute double numberValue;
  readonly attribute DOMString stringValue;
  readonly attribute boolean booleanValue;
  readonly attribute Node? singleNodeValue;
  readonly attribute boolean invalidIteratorState;
  readonly attribute unsigned long snapshotLength;
  Node? iterateNext();
  Node? snapshotItem(unsigned long index);
};
[Exposed=Window]
interface XPathNamespace : Node {
  readonly attribute DOMString prefix;
  readonly attribute DOMString localName;
  readonly attribute DOMString namespaceURI;
  const unsigned short XPATH_NAMESPACE_NODE = 13;
  readonly attribute Element ownerElement;
};
partial interface Document {
  XPathExpression createExpression(DOMString expression, optional XPathNSResolver? resolver = null);
  Node createNSResolver(Node nodeResolver);
  XPathResult evaluate(DOMString expression, Node contextNode, optional XPathNSResolver? resolver = null, optional unsigned short type = 0, optional XPathResult? result = null);
};


[Exposed=(Window,ServiceWorker), SecureContext]
interface NavigationPreloadManager {
  Promise<undefined> enable();
  Promise<undefined> disable();
  Promise<undefined> setHeaderValue(ByteString value);
  Promise<NavigationPreloadState> getState();
};
dictionary NavigationPreloadState {
  boolean enabled = false;
  ByteString headerValue = "true";
};


// UI Events composition interfaces; platform delivery uses native payloads.
[Exposed=Window]
interface CompositionEvent : UIEvent {
  constructor(DOMString type, optional CompositionEventInit eventInitDict = {});
  readonly attribute DOMString data;
  undefined initCompositionEvent(DOMString type, optional boolean bubbles = false, optional boolean cancelable = false, optional Window? view = null, optional DOMString data = "");
};
dictionary CompositionEventInit : UIEventInit {
  DOMString data = "";
};


[Exposed=Window]
interface MediaQueryList : EventTarget {
  readonly attribute DOMString media;
  readonly attribute boolean matches;
  attribute EventHandler onchange;
  undefined addListener(EventListener? callback);
  undefined removeListener(EventListener? callback);
};
[Exposed=Window]
interface MediaQueryListEvent : Event {
  constructor(DOMString type, optional MediaQueryListEventInit eventInitDict = {});
  readonly attribute DOMString media;
  readonly attribute boolean matches;
};
dictionary MediaQueryListEventInit : EventInit {
  DOMString media = "";
  boolean matches = false;
};

[Exposed=(Window,Worker)]
interface DOMStringList {
  readonly attribute unsigned long length;
  getter DOMString? item(unsigned long index);
  boolean contains(DOMString string);
};

[Exposed=Window]
interface HTMLGeolocationElement : HTMLElement {
  [HTMLConstructor] constructor();
  readonly attribute GeolocationPosition? position;
  readonly attribute GeolocationPositionError? error;
  [CEReactions, Reflect] attribute boolean autolocate;
  [CEReactions, Reflect] attribute boolean watch;
  readonly attribute boolean isValid;
  readonly attribute DOMString invalidReason;
  readonly attribute DOMString initialPermissionStatus;
  readonly attribute DOMString permissionStatus;
  attribute EventHandler onlocation;
  attribute EventHandler onpromptaction;
  attribute EventHandler onpromptdismiss;
  attribute EventHandler onvalidationstatuschange;
};
[Exposed=Window]
interface HTMLUserMediaElement : HTMLElement {
  [HTMLConstructor] constructor();
  static boolean isTypeSupported(DOMString type);
  [CEReactions] attribute DOMString type;
  readonly attribute boolean isValid;
  readonly attribute DOMString invalidReason;
  attribute EventHandler onpromptaction;
  attribute EventHandler onpromptdismiss;
  attribute EventHandler onvalidationstatuschange;
  attribute EventHandler onstream;
  attribute EventHandler oncancel;
  attribute EventHandler onerror;
  readonly attribute MediaStream? stream;
  readonly attribute DOMException? error;
  undefined setConstraints(optional HTMLMediaStreamConstraints constraints = {});
};

[Exposed=Window]
interface HTMLCameraElement : HTMLElement {
  [HTMLConstructor] constructor();
  readonly attribute MediaStreamTrack? track;
  readonly attribute DOMException? error;
  attribute EventHandler ontrack;
  attribute EventHandler oncancel;
  attribute EventHandler onerror;
  undefined setConstraints(optional MediaTrackConstraints constraints = {});
};
[Exposed=Window]
interface HTMLMicrophoneElement : HTMLElement {
  [HTMLConstructor] constructor();
  readonly attribute MediaStreamTrack? track;
  readonly attribute DOMException? error;
  attribute EventHandler ontrack;
  attribute EventHandler oncancel;
  attribute EventHandler onerror;
  undefined setConstraints(optional MediaTrackConstraints constraints = {});
};
[Exposed=Window]
interface HTMLInstallElement : HTMLElement {
  [HTMLConstructor] constructor();
  readonly attribute boolean isValid;
  readonly attribute DOMString invalidReason;
  attribute EventHandler onvalidationstatuschange;
};

dictionary HTMLMediaStreamConstraints { MediaTrackConstraints video; MediaTrackConstraints audio; };
[Exposed=Window, SecureContext]
interface GeolocationPosition {
  readonly attribute GeolocationCoordinates coords;
  readonly attribute double timestamp;
  object toJSON();
};
[Exposed=Window, SecureContext]
interface GeolocationCoordinates {
  readonly attribute double accuracy;
  readonly attribute double latitude;
  readonly attribute double longitude;
  readonly attribute double? altitude;
  readonly attribute double? altitudeAccuracy;
  readonly attribute double? heading;
  readonly attribute double? speed;
  object toJSON();
};
[Exposed=Window]
interface GeolocationPositionError {
  const unsigned short PERMISSION_DENIED = 1;
  const unsigned short POSITION_UNAVAILABLE = 2;
  const unsigned short TIMEOUT = 3;
  readonly attribute unsigned short code;
  readonly attribute DOMString message;
};

// Native implementation owns stream/track identities and EventTarget state.
[Exposed=Window]
interface MediaStream : EventTarget {
  constructor();
  constructor(MediaStream stream);
  constructor(sequence<MediaStreamTrack> tracks);
  readonly attribute DOMString id;
  readonly attribute boolean active;
  sequence<MediaStreamTrack> getAudioTracks();
  sequence<MediaStreamTrack> getVideoTracks();
  sequence<MediaStreamTrack> getTracks();
  MediaStreamTrack? getTrackById(DOMString trackId);
  undefined addTrack(MediaStreamTrack track);
  undefined removeTrack(MediaStreamTrack track);
  MediaStream clone();
  attribute EventHandler onaddtrack;
  attribute EventHandler onremovetrack;
};
[Exposed=Window]
interface MediaStreamTrack : EventTarget {
  readonly attribute DOMString kind;
  readonly attribute DOMString id;
  readonly attribute DOMString label;
  attribute boolean enabled;
  readonly attribute boolean muted;
  readonly attribute MediaStreamTrackState readyState;
  MediaStreamTrack clone();
  undefined stop();
  MediaTrackSettings getSettings();
  MediaTrackConstraints getConstraints();
  MediaTrackCapabilities getCapabilities();
  Promise<undefined> applyConstraints(optional MediaTrackConstraints constraints = {});
  attribute EventHandler onmute;
  attribute EventHandler onunmute;
  attribute EventHandler onended;
};

enum MediaStreamTrackState { "live", "ended" };
[Exposed=Window]
interface OverconstrainedError : DOMException {
  constructor(DOMString constraint, optional DOMString message = "");
  readonly attribute DOMString constraint;
};
dictionary DoubleRange { double max; double min; };
dictionary ULongRange { [Clamp] unsigned long max; [Clamp] unsigned long min; };
dictionary ConstrainDoubleRange : DoubleRange { double exact; double ideal; };
dictionary ConstrainULongRange : ULongRange { [Clamp] unsigned long exact; [Clamp] unsigned long ideal; };
dictionary ConstrainBooleanParameters { boolean exact; boolean ideal; };
dictionary ConstrainDOMStringParameters { (DOMString or sequence<DOMString>) exact; (DOMString or sequence<DOMString>) ideal; };
dictionary ConstrainBooleanOrDOMStringParameters { (boolean or DOMString) exact; (boolean or DOMString) ideal; };
typedef ([Clamp] unsigned long or ConstrainULongRange) ConstrainULong;
typedef (double or ConstrainDoubleRange) ConstrainDouble;
typedef (boolean or ConstrainBooleanParameters) ConstrainBoolean;
typedef (DOMString or sequence<DOMString> or ConstrainDOMStringParameters) ConstrainDOMString;
typedef (boolean or DOMString or ConstrainBooleanOrDOMStringParameters) ConstrainBooleanOrDOMString;
dictionary MediaTrackConstraintSet {
  ConstrainULong width; ConstrainULong height;
  ConstrainDouble aspectRatio; ConstrainDouble frameRate;
  ConstrainDOMString facingMode; ConstrainDOMString resizeMode;
  ConstrainULong sampleRate; ConstrainULong sampleSize;
  ConstrainBooleanOrDOMString echoCancellation;
  ConstrainBoolean autoGainControl; ConstrainBoolean noiseSuppression;
  ConstrainDouble latency; ConstrainULong channelCount;
  ConstrainDOMString deviceId; ConstrainDOMString groupId;
  ConstrainBoolean backgroundBlur;
};
dictionary MediaTrackConstraints : MediaTrackConstraintSet { sequence<MediaTrackConstraintSet> advanced; };
dictionary MediaTrackSettings {
  unsigned long width; unsigned long height; double aspectRatio; double frameRate;
  DOMString facingMode; DOMString resizeMode; unsigned long sampleRate; unsigned long sampleSize;
  (boolean or DOMString) echoCancellation; boolean autoGainControl; boolean noiseSuppression;
  double latency; unsigned long channelCount; DOMString deviceId; DOMString groupId; boolean backgroundBlur;
};
dictionary MediaTrackCapabilities {
  ULongRange width; ULongRange height; DoubleRange aspectRatio; DoubleRange frameRate;
  sequence<DOMString> facingMode; sequence<DOMString> resizeMode;
  ULongRange sampleRate; ULongRange sampleSize; sequence<(boolean or DOMString)> echoCancellation;
  sequence<boolean> autoGainControl; sequence<boolean> noiseSuppression;
  DoubleRange latency; ULongRange channelCount; DOMString deviceId; DOMString groupId;
  sequence<boolean> backgroundBlur;
};

[Exposed=Window]
interface MediaError {
  const unsigned short MEDIA_ERR_ABORTED = 1;
  const unsigned short MEDIA_ERR_NETWORK = 2;
  const unsigned short MEDIA_ERR_DECODE = 3;
  const unsigned short MEDIA_ERR_SRC_NOT_SUPPORTED = 4;
  readonly attribute unsigned short code;
  readonly attribute DOMString message;
};

// Native Navigation API surface follows the pinned HTML interface contract.
[Exposed=Window]
interface Navigation : EventTarget {
  sequence<NavigationHistoryEntry> entries();
  readonly attribute NavigationHistoryEntry? currentEntry;
  undefined updateCurrentEntry(NavigationUpdateCurrentEntryOptions options);
  readonly attribute NavigationTransition? transition;
  readonly attribute NavigationActivation? activation;

  readonly attribute boolean canGoBack;
  readonly attribute boolean canGoForward;

  NavigationResult navigate(USVString url, optional NavigationNavigateOptions options = {});
  NavigationResult reload(optional NavigationReloadOptions options = {});

  NavigationResult traverseTo(DOMString key, optional NavigationOptions options = {});
  NavigationResult back(optional NavigationOptions options = {});
  NavigationResult forward(optional NavigationOptions options = {});

  attribute EventHandler onnavigate;
  attribute EventHandler onnavigatesuccess;
  attribute EventHandler onnavigateerror;
  attribute EventHandler oncurrententrychange;
};

dictionary NavigationUpdateCurrentEntryOptions {
  required any state;
};

dictionary NavigationOptions {
  any info;
};

dictionary NavigationNavigateOptions : NavigationOptions {
  any state;
  NavigationHistoryBehavior history = "auto";
};

dictionary NavigationReloadOptions : NavigationOptions {
  any state;
};

dictionary NavigationResult {
  Promise<NavigationHistoryEntry> committed;
  Promise<NavigationHistoryEntry> finished;
};

enum NavigationHistoryBehavior {
  "auto",
  "push",
  "replace"
};

enum NavigationType {
 "push",
 "replace",
 "reload",
 "traverse"
};

[Exposed=Window]
interface NavigationHistoryEntry : EventTarget {
  readonly attribute USVString? url;
  readonly attribute DOMString key;
  readonly attribute DOMString id;
  readonly attribute long long index;
  readonly attribute boolean sameDocument;

  any getState();

  attribute EventHandler ondispose;
};

[Exposed=Window]
interface NavigationTransition {
  readonly attribute NavigationType navigationType;
  readonly attribute NavigationHistoryEntry from;
  readonly attribute NavigationDestination to;
  readonly attribute Promise<undefined> committed;
  readonly attribute Promise<undefined> finished;
};

[Exposed=Window]
interface NavigationActivation {
  readonly attribute NavigationHistoryEntry? from;
  readonly attribute NavigationHistoryEntry entry;
  readonly attribute NavigationType navigationType;
};

[Exposed=Window]
interface NavigateEvent : Event {
  constructor(DOMString type, NavigateEventInit eventInitDict);

  readonly attribute NavigationType navigationType;
  readonly attribute NavigationDestination destination;
  readonly attribute boolean canIntercept;
  readonly attribute boolean userInitiated;
  readonly attribute boolean hashChange;
  readonly attribute AbortSignal signal;
  readonly attribute FormData? formData;
  readonly attribute DOMString? downloadRequest;
  readonly attribute any info;
  readonly attribute boolean hasUAVisualTransition;
  readonly attribute Element? sourceElement;

  undefined intercept(optional NavigationInterceptOptions options = {});
  undefined scroll();
};

dictionary NavigateEventInit : EventInit {
  NavigationType navigationType = "push";
  required NavigationDestination destination;
  boolean canIntercept = false;
  boolean userInitiated = false;
  boolean hashChange = false;
  required AbortSignal signal;
  FormData? formData = null;
  DOMString? downloadRequest = null;
  any info;
  boolean hasUAVisualTransition = false;
  Element? sourceElement = null;
};

dictionary NavigationInterceptOptions {
  NavigationPrecommitHandler precommitHandler;
  NavigationInterceptHandler handler;
  NavigationFocusReset focusReset;
  NavigationScrollBehavior scroll;
};

enum NavigationFocusReset {
  "after-transition",
  "manual"
};

enum NavigationScrollBehavior {
  "after-transition",
  "manual"
};

callback NavigationInterceptHandler = Promise<undefined> ();

[Exposed=Window]
interface NavigationPrecommitController {
  undefined redirect(USVString url, optional NavigationNavigateOptions options = {});
  undefined addHandler(NavigationInterceptHandler handler);
};

callback NavigationPrecommitHandler = Promise<undefined> (NavigationPrecommitController controller);

[Exposed=Window]
interface NavigationDestination {
  readonly attribute USVString url;
  readonly attribute DOMString key;
  readonly attribute DOMString id;
  readonly attribute long long index;
  readonly attribute boolean sameDocument;

  any getState();
};

[Exposed=Window]
interface NavigationCurrentEntryChangeEvent : Event {
  constructor(DOMString type, NavigationCurrentEntryChangeEventInit eventInitDict);

  readonly attribute NavigationType? navigationType;
  readonly attribute NavigationHistoryEntry from;
};

dictionary NavigationCurrentEntryChangeEventInit : EventInit {
  NavigationType? navigationType = null;
  required NavigationHistoryEntry from;
};


[Exposed=(DedicatedWorker,SharedWorker)]
interface FileReaderSync {
    constructor();
    ArrayBuffer readAsArrayBuffer(Blob blob);
    DOMString readAsBinaryString(Blob blob);
    DOMString readAsText(Blob blob, optional DOMString encoding);
    DOMString readAsDataURL(Blob blob);
};

dictionary GetRootNodeOptions {
  boolean composed = false;
};

[Exposed=(Window,Worker)]
interface Crypto {
  [SecureContext, SameObject] readonly attribute SubtleCrypto subtle;
  ArrayBufferView getRandomValues(ArrayBufferView array);
  [SecureContext] DOMString randomUUID();
};
[SecureContext, Exposed=(Window,Worker)]
interface SubtleCrypto {
  Promise<(CryptoKey or CryptoKeyPair)> generateKey(AlgorithmIdentifier algorithm, boolean extractable, sequence<KeyUsage> keyUsages);
  Promise<CryptoKey> importKey(DOMString format, any keyData, AlgorithmIdentifier algorithm, boolean extractable, sequence<KeyUsage> keyUsages);
  Promise<(ArrayBuffer or JsonWebKey)> exportKey(DOMString format, CryptoKey key);
  Promise<ArrayBuffer> encrypt(AlgorithmIdentifier algorithm, CryptoKey key, (ArrayBuffer or ArrayBufferView) data);
  Promise<ArrayBuffer> decrypt(AlgorithmIdentifier algorithm, CryptoKey key, (ArrayBuffer or ArrayBufferView) data);
  Promise<ArrayBuffer> digest(AlgorithmIdentifier algorithm, (ArrayBuffer or ArrayBufferView) data);
  Promise<ArrayBuffer> wrapKey(DOMString format, CryptoKey key, CryptoKey wrappingKey, AlgorithmIdentifier wrapAlgorithm);
  Promise<CryptoKey> unwrapKey(DOMString format, (ArrayBuffer or ArrayBufferView) wrappedKey, CryptoKey unwrappingKey, AlgorithmIdentifier unwrapAlgorithm, AlgorithmIdentifier unwrappedKeyAlgorithm, boolean extractable, sequence<KeyUsage> keyUsages);
  Promise<ArrayBuffer> sign(AlgorithmIdentifier algorithm, CryptoKey key, (ArrayBuffer or ArrayBufferView) data);
  Promise<boolean> verify(AlgorithmIdentifier algorithm, CryptoKey key, (ArrayBuffer or ArrayBufferView) signature, (ArrayBuffer or ArrayBufferView) data);
  Promise<ArrayBuffer> deriveBits(AlgorithmIdentifier algorithm, CryptoKey baseKey, optional unsigned long? length);
  Promise<CryptoKey> deriveKey(AlgorithmIdentifier algorithm, CryptoKey baseKey, AlgorithmIdentifier derivedKeyType, boolean extractable, sequence<KeyUsage> keyUsages);
};
[SecureContext, Serializable, Exposed=(Window,Worker)]
interface CryptoKey {
  readonly attribute KeyType type;
  readonly attribute boolean extractable;
  readonly attribute object algorithm;
  readonly attribute object usages;
};

typedef (object or DOMString) AlgorithmIdentifier;
enum KeyType { "public", "private", "secret" };
enum KeyUsage { "encrypt", "decrypt", "sign", "verify", "deriveKey", "deriveBits", "wrapKey", "unwrapKey" };
dictionary CryptoKeyPair { CryptoKey publicKey; CryptoKey privateKey; };
dictionary JsonWebKey {
  DOMString kty;
  DOMString use;
  sequence<DOMString> key_ops;
  DOMString alg;
  boolean ext;
  DOMString crv;
  DOMString x;
  DOMString d;
  DOMString k;
};

[Exposed=Window]
interface Animation : EventTarget {
  constructor(optional AnimationEffect? effect = null, optional AnimationTimeline? timeline);
  attribute DOMString id;
  attribute EventHandler onfinish;
  attribute EventHandler oncancel;
  attribute EventHandler onremove;
};
[Exposed=Window] interface CSSAnimation : Animation {};
[Exposed=Window] interface CSSTransition : Animation {};
[Exposed=Window] interface AnimationTimeline {};
[Exposed=Window] interface DocumentTimeline : AnimationTimeline { constructor(optional any options); };
[Exposed=Window] interface AnimationEffect {};
[Exposed=Window] interface KeyframeEffect : AnimationEffect { constructor(any target, optional any keyframes, optional any options); };
[Exposed=Window] interface AnimationEvent : Event { constructor(DOMString type, optional any eventInitDict); };
[Exposed=Window] interface TransitionEvent : Event { constructor(DOMString type, optional any eventInitDict); };
[Exposed=Window] interface AnimationPlaybackEvent : Event { constructor(DOMString type, optional any eventInitDict); };

[Exposed=(Window,Worker)]
interface Notification : EventTarget {
  constructor(DOMString title, optional any options);
  attribute EventHandler onclick;
  attribute EventHandler onshow;
  attribute EventHandler onerror;
  attribute EventHandler onclose;
  readonly attribute DOMString title;
  readonly attribute DOMString dir;
  readonly attribute DOMString lang;
  readonly attribute DOMString body;
  readonly attribute USVString navigate;
  readonly attribute DOMString tag;
  readonly attribute USVString image;
  readonly attribute USVString icon;
  readonly attribute USVString badge;
  readonly attribute any vibrate;
  readonly attribute double timestamp;
  readonly attribute boolean renotify;
  readonly attribute boolean? silent;
  readonly attribute boolean requireInteraction;
  readonly attribute any data;
  readonly attribute any actions;
  undefined close();
};

[Exposed=(Window,DedicatedWorker)]
interface MediaSource : EventTarget {
  constructor();
  [SameObject, Exposed=DedicatedWorker] readonly attribute MediaSourceHandle handle;
  readonly attribute DOMString readyState;
  readonly attribute unrestricted double duration;
};
[Exposed=(Window,DedicatedWorker)]
interface ManagedMediaSource : MediaSource {
  constructor();
  readonly attribute boolean streaming;
};


[Exposed=(Window,DedicatedWorker)]
interface MediaSourceHandle {};

interface mixin InterestInvokerElement {
  [CEReactions, Reflect=interestfor] attribute Element? interestForElement;
};
HTMLAnchorElement includes InterestInvokerElement;
HTMLAreaElement includes InterestInvokerElement;
HTMLButtonElement includes InterestInvokerElement;
SVGAElement includes InterestInvokerElement;
[Exposed=Window]
interface InterestEvent : Event {
  constructor(DOMString type, optional InterestEventInit eventInitDict = {});
  readonly attribute Element? source;
};
dictionary InterestEventInit : EventInit { Element? source = null; };

[Exposed=Window]
interface HTMLSubmitButtonBehavior {
  constructor();
  [CEReactions] attribute boolean disabled;
  readonly attribute HTMLFormElement? form;
  [CEReactions] attribute DOMString formAction;
  [CEReactions] attribute DOMString formEnctype;
  [CEReactions] attribute DOMString formMethod;
  [CEReactions] attribute boolean formNoValidate;
  [CEReactions] attribute DOMString formTarget;
  readonly attribute NodeList labels;
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute DOMString value;
};
dictionary ElementInternalsOptions {
  sequence<HTMLSubmitButtonBehavior> behaviors = [];
};

[Exposed=Window]
interface HTMLPictureElement : HTMLElement {};
[Exposed=Window]
interface HTMLSourceElement : HTMLElement {
  [CEReactions] attribute USVString src;
  [CEReactions] attribute DOMString type;
  [CEReactions] attribute DOMString srcset;
  [CEReactions] attribute DOMString sizes;
  [CEReactions] attribute DOMString media;
  [CEReactions] attribute unsigned long width;
  [CEReactions] attribute unsigned long height;
};


[Exposed=(Window,Worker)]
interface Performance : EventTarget {
  readonly attribute double timeOrigin;
  double now();
  sequence<PerformanceEntry> getEntries();
  sequence<PerformanceEntry> getEntriesByType(DOMString type);
  sequence<PerformanceEntry> getEntriesByName(DOMString name, optional DOMString type);
  undefined clearResourceTimings();
  undefined setResourceTimingBufferSize(unsigned long maxSize);
  object toJSON();
};
[Exposed=(Window,Worker)]
interface PerformanceEntry {
  readonly attribute DOMString name;
  readonly attribute DOMString entryType;
  readonly attribute double startTime;
  readonly attribute double duration;
  object toJSON();
};
callback PerformanceObserverCallback = undefined (PerformanceObserverEntryList entries, PerformanceObserver observer, optional PerformanceObserverCallbackOptions options = {});
dictionary PerformanceObserverCallbackOptions { unsigned long long droppedEntriesCount; };
dictionary PerformanceObserverInit {
  boolean buffered;
  double durationThreshold;
  sequence<DOMString> entryTypes;
  DOMString type;
};
[Exposed=(Window,Worker)]
interface PerformanceObserver {
  constructor(PerformanceObserverCallback callback);
  static readonly attribute FrozenArray<DOMString> supportedEntryTypes;
  undefined observe(PerformanceObserverInit options);
  undefined disconnect();
  sequence<PerformanceEntry> takeRecords();
};
[Exposed=(Window,Worker)]
interface PerformanceObserverEntryList {
  sequence<PerformanceEntry> getEntries();
  sequence<PerformanceEntry> getEntriesByType(DOMString type);
  sequence<PerformanceEntry> getEntriesByName(DOMString name, optional DOMString type);
};
[Exposed=Window]
interface PerformancePaintTiming : PerformanceEntry {};
[Exposed=(Window,Worker)]
interface PerformanceResourceTiming : PerformanceEntry {
  readonly attribute FrozenArray<PerformanceServerTiming> serverTiming;
  readonly attribute DOMString initiatorType;
  readonly attribute DOMString nextHopProtocol;
  readonly attribute DOMString deliveryType;
  readonly attribute DOMString renderBlockingStatus;
  readonly attribute DOMString contentType;
  readonly attribute double workerStart;
  readonly attribute double redirectStart;
  readonly attribute double redirectEnd;
  readonly attribute double fetchStart;
  readonly attribute double domainLookupStart;
  readonly attribute double domainLookupEnd;
  readonly attribute double connectStart;
  readonly attribute double connectEnd;
  readonly attribute double secureConnectionStart;
  readonly attribute double requestStart;
  readonly attribute double responseStart;
  readonly attribute double responseEnd;
  readonly attribute unsigned long long transferSize;
  readonly attribute unsigned long long encodedBodySize;
  readonly attribute unsigned long long decodedBodySize;
  readonly attribute unsigned short responseStatus;
};

[Exposed=(Window,Worker)]
interface PerformanceServerTiming {
  readonly attribute DOMString name;
  readonly attribute DOMString description;
  readonly attribute DOMHighResTimeStamp duration;
  object toJSON();
};

[Exposed=Window]
interface PerformanceNavigationTiming : PerformanceResourceTiming {
  readonly attribute DOMString type;
  readonly attribute unsigned short redirectCount;
  readonly attribute double unloadEventStart;
  readonly attribute double unloadEventEnd;
  readonly attribute double domInteractive;
  readonly attribute double domContentLoadedEventStart;
  readonly attribute double domContentLoadedEventEnd;
  readonly attribute double domComplete;
  readonly attribute double loadEventStart;
  readonly attribute double loadEventEnd;
  readonly attribute double activationStart;
  readonly attribute double criticalCHRestart;
};

partial interface HTMLButtonElement {
  [CEReactions] attribute DOMString command;
  [CEReactions] attribute Element? commandForElement;
};
[Exposed=Window]
interface CommandEvent : Event {
  constructor(DOMString type, optional CommandEventInit eventInitDict = {});
  readonly attribute DOMString command;
  readonly attribute Element? source;
};
dictionary CommandEventInit : EventInit {
  DOMString command = "";
  Element? source = null;
};

// Fullscreen state is shared by script requests and native command invokers.
partial interface Element {
  Promise<undefined> requestFullscreen(optional FullscreenOptions options = {});
};
enum FullscreenNavigationUI { "auto", "show", "hide" };
dictionary FullscreenOptions { FullscreenNavigationUI navigationUI = "auto"; };
partial interface Document {
  readonly attribute boolean fullscreenEnabled;
  readonly attribute Element? fullscreenElement;
  Promise<undefined> exitFullscreen();
  attribute EventListener? onfullscreenchange;
  attribute EventListener? onfullscreenerror;
};

partial interface ShadowRoot { readonly attribute Element? fullscreenElement; };
partial interface Element { attribute EventListener? onfullscreenchange; attribute EventListener? onfullscreenerror; };

partial interface HTMLVideoElement {
  Promise<PictureInPictureWindow> requestPictureInPicture();
  [CEReactions] attribute boolean disablePictureInPicture;
  attribute EventListener? onenterpictureinpicture;
  attribute EventListener? onleavepictureinpicture;
};
partial interface Document {
  readonly attribute boolean pictureInPictureEnabled;
  readonly attribute Element? pictureInPictureElement;
  Promise<undefined> exitPictureInPicture();
};
partial interface ShadowRoot { readonly attribute Element? pictureInPictureElement; };
[Exposed=Window] interface PictureInPictureWindow : EventTarget {
  readonly attribute long width;
  readonly attribute long height;
  attribute EventListener? onresize;
};
[Exposed=Window] interface PictureInPictureEvent : Event {
  constructor(DOMString type, PictureInPictureEventInit eventInitDict);
  readonly attribute PictureInPictureWindow pictureInPictureWindow;
};
dictionary PictureInPictureEventInit : EventInit {
  required PictureInPictureWindow pictureInPictureWindow;
};

[Exposed=Window] interface PermissionsPolicy {
  boolean allowsFeature(DOMString feature, optional DOMString origin);
  sequence<DOMString> features();
  sequence<DOMString> allowedFeatures();
  sequence<DOMString> getAllowlistForFeature(DOMString feature);
};
partial interface Document { [SameObject] readonly attribute PermissionsPolicy permissionsPolicy; };
partial interface HTMLIFrameElement { [SameObject] readonly attribute PermissionsPolicy permissionsPolicy; };

[Exposed=Window]
interface BaseAudioContext : EventTarget {
 readonly attribute float sampleRate;
 readonly attribute double currentTime;
 readonly attribute DOMString state;
 [SameObject] readonly attribute AudioWorklet audioWorklet;
};
[Exposed=Window]
interface OfflineAudioContext : BaseAudioContext {
 constructor(unsigned long numberOfChannels, unsigned long length, float sampleRate);
 readonly attribute unsigned long length;
};
[Exposed=Window]
interface AudioWorklet { Promise<undefined> addModule(USVString moduleURL, optional object options); };
[Exposed=Window]
interface AudioWorkletNode : EventTarget {
 attribute EventListener? onprocessorerror;
 constructor(BaseAudioContext context, DOMString name, optional object options);
 readonly attribute MessagePort port;
};

[Exposed=Window]
interface AudioContext : BaseAudioContext {
 constructor(optional object options);
 Promise<undefined> close();
};

// Native URL-reflection interfaces used by document resource selection.
[Exposed=Window]
interface HTMLEmbedElement : HTMLElement {
  Document? getSVGDocument();
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute DOMString width;
  [CEReactions] attribute DOMString height;
  [CEReactions] attribute USVString src;
  [CEReactions] attribute DOMString type;
};
[Exposed=Window]
interface HTMLTrackElement : HTMLElement {
  [HTMLConstructor] constructor();
  [CEReactions] attribute USVString src;
  [CEReactions] attribute DOMString kind;
  [CEReactions] attribute DOMString srclang;
  [CEReactions] attribute DOMString label;
  [CEReactions] attribute boolean default;
  const unsigned short NONE = 0;
  const unsigned short LOADING = 1;
  const unsigned short LOADED = 2;
  const unsigned short ERROR = 3;
  readonly attribute unsigned short readyState;
  [SameObject] readonly attribute TextTrack track;
  attribute EventListener? oncuechange;
};
enum TextTrackMode { "disabled", "hidden", "showing" };
enum TextTrackKind { "subtitles", "captions", "descriptions", "chapters", "metadata" };
[Exposed=Window]
interface TextTrack : EventTarget {
  readonly attribute TextTrackKind kind;
  readonly attribute DOMString label;
  readonly attribute DOMString language;
  readonly attribute DOMString id;
  readonly attribute DOMString inBandMetadataTrackDispatchType;
  attribute TextTrackMode mode;
  readonly attribute TextTrackCueList? cues;
  readonly attribute TextTrackCueList? activeCues;
  undefined addCue(TextTrackCue cue);
  undefined removeCue(TextTrackCue cue);
  attribute EventListener? oncuechange;
};
[Exposed=Window]
interface TextTrackList : EventTarget {
  readonly attribute unsigned long length;
  getter TextTrack (unsigned long index);
  TextTrack? getTrackById(DOMString id);
  attribute EventListener? onchange;
  attribute EventListener? onaddtrack;
  attribute EventListener? onremovetrack;
};
[Exposed=Window]
interface TextTrackCueList {
  readonly attribute unsigned long length;
  getter TextTrackCue (unsigned long index);
  TextTrackCue? getCueById(DOMString id);
};
[Exposed=Window]
interface TextTrackCue : EventTarget {
  readonly attribute TextTrack? track;
  attribute DOMString id;
  attribute double startTime;
  attribute unrestricted double endTime;
  attribute boolean pauseOnExit;
  attribute EventListener? onenter;
  attribute EventListener? onexit;
};
enum AutoKeyword { "auto" };
typedef (double or AutoKeyword) LineAndPositionSetting;
enum DirectionSetting { "", "rl", "lr" };
enum LineAlignSetting { "start", "center", "end" };
enum PositionAlignSetting { "line-left", "center", "line-right", "auto" };
enum AlignSetting { "start", "center", "end", "left", "right" };
enum ScrollSetting { "", "up" };
[Exposed=Window]
interface VTTCue : TextTrackCue {
  constructor(double startTime, unrestricted double endTime, DOMString text);
  attribute VTTRegion? region;
  attribute DirectionSetting vertical;
  attribute boolean snapToLines;
  attribute LineAndPositionSetting line;
  attribute LineAlignSetting lineAlign;
  attribute LineAndPositionSetting position;
  attribute PositionAlignSetting positionAlign;
  attribute double size;
  attribute AlignSetting align;
  attribute DOMString text;
  DocumentFragment getCueAsHTML();
};
[Exposed=Window]
interface VTTRegion {
  constructor();
  attribute DOMString id;
  attribute double width;
  attribute unsigned long lines;
  attribute double regionAnchorX;
  attribute double regionAnchorY;
  attribute double viewportAnchorX;
  attribute double viewportAnchorY;
  attribute ScrollSetting scroll;
};

[Exposed=Window]
interface HTMLFrameElement : HTMLElement {
  [CEReactions] attribute DOMString name;
  [CEReactions] attribute USVString src;
  readonly attribute Document? contentDocument;
  readonly attribute Window? contentWindow;
};

[Exposed=Window]
interface TrackEvent : Event {
  constructor(DOMString type, optional TrackEventInit eventInitDict = {});
  readonly attribute (VideoTrack or AudioTrack or TextTrack)? track;
};
dictionary TrackEventInit : EventInit {
  (VideoTrack or AudioTrack or TextTrack)? track = null;
};

[Exposed=Window]
interface AudioTrackList : EventTarget {
  readonly attribute unsigned long length;
  getter AudioTrack (unsigned long index);
  AudioTrack? getTrackById(DOMString id);
  attribute EventListener? onchange;
  attribute EventListener? onaddtrack;
  attribute EventListener? onremovetrack;
};
[Exposed=Window]
interface AudioTrack {
  readonly attribute DOMString id;
  readonly attribute DOMString kind;
  readonly attribute DOMString label;
  readonly attribute DOMString language;
  attribute boolean enabled;
};
[Exposed=Window]
interface VideoTrackList : EventTarget {
  readonly attribute unsigned long length;
  getter VideoTrack (unsigned long index);
  VideoTrack? getTrackById(DOMString id);
  readonly attribute long selectedIndex;
  attribute EventListener? onchange;
  attribute EventListener? onaddtrack;
  attribute EventListener? onremovetrack;
};
[Exposed=Window]
interface VideoTrack {
  readonly attribute DOMString id;
  readonly attribute DOMString kind;
  readonly attribute DOMString label;
  readonly attribute DOMString language;
  attribute boolean selected;
};


// Native rendering-update observations.
enum ResizeObserverBoxOptions { "border-box", "content-box", "device-pixel-content-box" };
dictionary ResizeObserverOptions { ResizeObserverBoxOptions box = "content-box"; };
callback ResizeObserverCallback = undefined (sequence<ResizeObserverEntry> entries, ResizeObserver observer);
[Exposed=Window] interface ResizeObserver {
  constructor(ResizeObserverCallback callback);
  undefined observe(Element target, optional ResizeObserverOptions options = {});
  undefined unobserve(Element target);
  undefined disconnect();
};
[Exposed=Window] interface ResizeObserverEntry {
  readonly attribute Element target;
  readonly attribute DOMRectReadOnly contentRect;
  readonly attribute FrozenArray<ResizeObserverSize> borderBoxSize;
  readonly attribute FrozenArray<ResizeObserverSize> contentBoxSize;
  readonly attribute FrozenArray<ResizeObserverSize> devicePixelContentBoxSize;
};
[Exposed=Window] interface ResizeObserverSize {
  readonly attribute unrestricted double inlineSize;
  readonly attribute unrestricted double blockSize;
};

callback IntersectionObserverCallback = undefined (sequence<IntersectionObserverEntry> entries, IntersectionObserver observer);
dictionary IntersectionObserverInit {
  (Element or Document)? root = null;
  DOMString rootMargin = "0px";
  DOMString scrollMargin = "0px";
  (double or sequence<double>) threshold = 0;
  long delay = 0;
  boolean trackVisibility = false;
};
[Exposed=Window] interface IntersectionObserver {
  constructor(IntersectionObserverCallback callback, optional IntersectionObserverInit options = {});
  readonly attribute (Element or Document)? root;
  readonly attribute DOMString rootMargin;
  readonly attribute DOMString scrollMargin;
  readonly attribute FrozenArray<double> thresholds;
  readonly attribute long delay;
  readonly attribute boolean trackVisibility;
  undefined observe(Element target);
  undefined unobserve(Element target);
  undefined disconnect();
  sequence<IntersectionObserverEntry> takeRecords();
};
dictionary IntersectionObserverEntryInit {
  required DOMHighResTimeStamp time;
  required DOMRectInit? rootBounds;
  required DOMRectInit boundingClientRect;
  required DOMRectInit intersectionRect;
  required boolean isIntersecting;
  required boolean isVisible;
  required double intersectionRatio;
  required Element target;
};
[Exposed=Window] interface IntersectionObserverEntry {
  constructor(IntersectionObserverEntryInit intersectionObserverEntryInit);
  readonly attribute DOMHighResTimeStamp time;
  readonly attribute DOMRectReadOnly? rootBounds;
  readonly attribute DOMRectReadOnly boundingClientRect;
  readonly attribute DOMRectReadOnly intersectionRect;
  readonly attribute boolean isIntersecting;
  readonly attribute boolean isVisible;
  readonly attribute double intersectionRatio;
  readonly attribute Element target;
};

// Storage Access API; receiver ownership and Promise settlement are native.
partial interface Document { Promise<boolean> hasStorageAccess(); Promise<undefined> requestStorageAccess(); };

// Native foreground and active-document focus inspection.
partial interface Document { boolean hasFocus(); };

// IndexedDB extends the HTML global mixin; the base worker interface is not [Global].
interface mixin WindowOrWorkerGlobalScope {
  [SameObject] readonly attribute IDBFactory indexedDB;
};
Window includes WindowOrWorkerGlobalScope;
WorkerGlobalScope includes WindowOrWorkerGlobalScope;

[Global=(Worker,DedicatedWorker), Exposed=DedicatedWorker]
interface DedicatedWorkerGlobalScope : WorkerGlobalScope {};
[Global=(Worker,SharedWorker), Exposed=SharedWorker]
interface SharedWorkerGlobalScope : WorkerGlobalScope {};
[Global=(Worker,ServiceWorker), Exposed=ServiceWorker]
interface ServiceWorkerGlobalScope : WorkerGlobalScope {};

partial interface HTMLElement {
  attribute EditContext? editContext;
};
dictionary TextUpdateEventInit : EventInit {
  unsigned long updateRangeStart = 0;
  unsigned long updateRangeEnd = 0;
  DOMString text = "";
  unsigned long selectionStart = 0;
  unsigned long selectionEnd = 0;
};
[Exposed=Window]
interface TextUpdateEvent : Event {
  constructor(DOMString type, optional TextUpdateEventInit options = {});
  readonly attribute unsigned long updateRangeStart;
  readonly attribute unsigned long updateRangeEnd;
  readonly attribute DOMString text;
  readonly attribute unsigned long selectionStart;
  readonly attribute unsigned long selectionEnd;
};
dictionary EditContextInit {
  DOMString text = "";
  unsigned long selectionStart = 0;
  unsigned long selectionEnd = 0;
};
[Exposed=Window]
interface EditContext : EventTarget {
  constructor(optional EditContextInit options = {});
  undefined updateText(unsigned long rangeStart, unsigned long rangeEnd, DOMString text);
  undefined updateSelection(unsigned long start, unsigned long end);
  undefined updateControlBounds(DOMRect controlBounds);
  undefined updateSelectionBounds(DOMRect selectionBounds);
  undefined updateCharacterBounds(unsigned long rangeStart, sequence<DOMRect> characterBounds);
  sequence<HTMLElement> attachedElements();
  readonly attribute DOMString text;
  readonly attribute unsigned long selectionStart;
  readonly attribute unsigned long selectionEnd;
  readonly attribute unsigned long characterBoundsRangeStart;
  sequence<DOMRect> characterBounds();
  attribute EventHandler ontextupdate;
  attribute EventHandler ontextformatupdate;
  attribute EventHandler oncharacterboundsupdate;
  attribute EventHandler oncompositionstart;
  attribute EventHandler oncompositionend;
};

enum UnderlineStyle { "none", "solid", "dotted", "dashed", "wavy" };
enum UnderlineThickness { "none", "thin", "thick" };
dictionary TextFormatInit {
  unsigned long rangeStart = 0;
  unsigned long rangeEnd = 0;
  UnderlineStyle underlineStyle = "none";
  UnderlineThickness underlineThickness = "none";
};
[Exposed=Window]
interface TextFormat {
  constructor(optional TextFormatInit options = {});
  readonly attribute unsigned long rangeStart;
  readonly attribute unsigned long rangeEnd;
  readonly attribute UnderlineStyle underlineStyle;
  readonly attribute UnderlineThickness underlineThickness;
};
dictionary TextFormatUpdateEventInit : EventInit { sequence<TextFormat> textFormats = []; };
[Exposed=Window]
interface TextFormatUpdateEvent : Event {
  constructor(DOMString type, optional TextFormatUpdateEventInit options = {});
  sequence<TextFormat> getTextFormats();
};
dictionary CharacterBoundsUpdateEventInit : EventInit {
  unsigned long rangeStart = 0;
  unsigned long rangeEnd = 0;
};
[Exposed=Window]
interface CharacterBoundsUpdateEvent : Event {
  constructor(DOMString type, optional CharacterBoundsUpdateEventInit options = {});
  readonly attribute unsigned long rangeStart;
  readonly attribute unsigned long rangeEnd;
};
