//! Production executable page backed by the Rust-owned live DOM.
//!
//! This module never exports or reconstructs a JavaScript-owned node graph.

#[cfg(test)]
use std::time::Duration;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use base64::Engine as _;
use oryn_common::v3::SemanticAction;
use oryn_dom::{DomRuntime, NodeId, NodeKind, html};
use oryn_js::realm::RealmId;
use serde::{Deserialize, Serialize};

use crate::{
    host::{HostError, JavaScriptHost, RawV8Host},
    html::ParsedDocument,
    network::{NetworkBroker, SharedNetworkBroker},
};
use url::Url;

const RUST_RUNTIME_GLOBALS: &str = r#"
globalThis.window=globalThis;
globalThis.self=globalThis;
globalThis.global=globalThis;
globalThis.queueMicrotask=callback=>Promise.resolve().then(callback);
globalThis.__oryn_console=[];
globalThis.console={
  log:(...values)=>__oryn_console.push({level:'log',message:values.map(String).join(' ')}),
  info:(...values)=>__oryn_console.push({level:'info',message:values.map(String).join(' ')}),
  warn:(...values)=>__oryn_console.push({level:'warn',message:values.map(String).join(' ')}),
  error:(...values)=>__oryn_console.push({level:'error',message:values.map(String).join(' ')})
};
globalThis.__oryn_take_console=()=>__oryn_console.splice(0);
globalThis.alert=value=>{__oryn_console.push({level:'info',message:'alert: '+String(value)})};
globalThis.confirm=value=>{__oryn_console.push({level:'info',message:'confirm dismissed: '+String(value)});return false};
globalThis.prompt=value=>{__oryn_console.push({level:'info',message:'prompt dismissed: '+String(value)});return null};
globalThis.__oryn_context_trace=[];
globalThis.__oryn_take_context_trace=()=>__oryn_context_trace.splice(0);
globalThis.__oryn_custom_reactions=[];
globalThis.__oryn_run_custom_reactions=()=>{for(let rounds=0;__oryn_custom_reactions.length&&rounds<1024;rounds++){const reactions=__oryn_custom_reactions.splice(0);for(const reaction of reactions)reaction()}if(__oryn_custom_reactions.length)throw new Error('custom-element reaction limit exceeded')};
globalThis.__oryn_custom_definitions=new Map();
const __oryn_upgrade_custom_element=node=>{if(!node||!node.tagName)return false;const definition=__oryn_custom_definitions.get(String(node.tagName).toLowerCase());if(!definition||node.__orynCustomDefinition===definition)return false;node.__orynCustomDefinition=definition;try{Object.setPrototypeOf(node,definition.prototype);const constructed=Reflect.construct(definition,[],definition);for(const key of Reflect.ownKeys(constructed))try{node[key]=constructed[key]}catch(_){}}catch(error){__oryn_console.push({level:'error',message:'custom element constructor: '+String(error)})}return true};
const __oryn_queue_connected=node=>{if(node&&typeof node.connectedCallback==='function'&&!node.__orynConnected){node.__orynConnected=true;__oryn_custom_reactions.push(()=>node.connectedCallback())}};
globalThis.customElements={define(name,constructor){name=String(name).toLowerCase();if(!name.includes('-'))throw new Error('custom element name must contain a hyphen');if(__oryn_custom_definitions.has(name))throw new Error('custom element already defined');__oryn_custom_definitions.set(name,constructor);for(const node of document.querySelectorAll(name))if(__oryn_upgrade_custom_element(node))__oryn_queue_connected(node)},get(name){return __oryn_custom_definitions.get(String(name).toLowerCase())},whenDefined(name){name=String(name).toLowerCase();if(this.get(name))return Promise.resolve(this.get(name));return new Promise(resolve=>{const timer=setInterval(()=>{if(this.get(name)){clearInterval(timer);resolve(this.get(name))}},1)})},upgrade(root){const apply=node=>{if(__oryn_upgrade_custom_element(node))__oryn_queue_connected(node)};if(root&&root.tagName)apply(root);if(root&&root.querySelectorAll)for(const node of root.querySelectorAll('*'))apply(node)}};
for(const [owner,method] of [[Document.prototype,'createElement'],[Node.prototype,'appendChild'],[Node.prototype,'insertBefore'],[Node.prototype,'replaceChild']]){const native=owner[method];if(typeof native!=='function')continue;Object.defineProperty(owner,method,{configurable:true,writable:true,value:function(...args){const result=native.apply(this,args);const node=method==='createElement'?result:args[0];__oryn_upgrade_custom_element(node);if(method!=='createElement')__oryn_queue_connected(node);return result}})}
globalThis.__oryn_animation_frames=new Map();globalThis.__oryn_next_animation_frame=1;
globalThis.requestAnimationFrame=callback=>{const id=__oryn_next_animation_frame++;__oryn_animation_frames.set(id,callback);return id};
globalThis.cancelAnimationFrame=id=>__oryn_animation_frames.delete(Number(id));
globalThis.__oryn_run_animation_frames=()=>{const callbacks=[...__oryn_animation_frames.values()];__oryn_animation_frames.clear();const timestamp=Date.now();for(const callback of callbacks)callback(timestamp)};
globalThis.__oryn_resize_observers=new Set();globalThis.__oryn_intersection_observers=new Set();
class OrynResizeObserver{constructor(callback){if(typeof callback!=='function')throw new TypeError('callback must be callable');this.callback=callback;this.targets=new Map();__oryn_resize_observers.add(this)}observe(target){this.targets.set(target,null)}unobserve(target){this.targets.delete(target)}disconnect(){this.targets.clear();__oryn_resize_observers.delete(this)}}
class OrynIntersectionObserver{constructor(callback,options={}){if(typeof callback!=='function')throw new TypeError('callback must be callable');this.callback=callback;this.root=options.root||null;this.rootMargin=String(options.rootMargin||'0px');this.thresholds=Array.isArray(options.threshold)?options.threshold.map(Number):[Number(options.threshold||0)];this.targets=new Map();__oryn_intersection_observers.add(this)}observe(target){this.targets.set(target,null)}unobserve(target){this.targets.delete(target)}disconnect(){this.targets.clear();__oryn_intersection_observers.delete(this)}takeRecords(){return []}}
globalThis.ResizeObserver=OrynResizeObserver;globalThis.IntersectionObserver=OrynIntersectionObserver;
const __oryn_rect_key=rect=>[rect.x,rect.y,rect.width,rect.height].join(':');
globalThis.__oryn_run_geometry_observers=()=>{for(const observer of __oryn_resize_observers){const entries=[];for(const target of observer.targets.keys())try{const rect=target.getBoundingClientRect(),key=__oryn_rect_key(rect);if(observer.targets.get(target)!==key){observer.targets.set(target,key);entries.push({target,contentRect:rect,borderBoxSize:[{inlineSize:rect.width,blockSize:rect.height}],contentBoxSize:[{inlineSize:rect.width,blockSize:rect.height}]})}}catch(_){}if(entries.length)observer.callback(entries,observer)}for(const observer of __oryn_intersection_observers){const entries=[];for(const target of observer.targets.keys())try{const rect=target.getBoundingClientRect(),viewport={x:0,y:0,width:innerWidth||0,height:innerHeight||0},width=Math.max(0,Math.min(rect.right,viewport.width)-Math.max(rect.left,0)),height=Math.max(0,Math.min(rect.bottom,viewport.height)-Math.max(rect.top,0)),area=rect.width*rect.height,ratio=area>0?(width*height)/area:0,key=__oryn_rect_key(rect)+':'+ratio;if(observer.targets.get(target)!==key){observer.targets.set(target,key);entries.push({target,boundingClientRect:rect,intersectionRect:{x:Math.max(rect.x,0),y:Math.max(rect.y,0),width,height},rootBounds:viewport,isIntersecting:ratio>0,intersectionRatio:ratio,time:Date.now()})}}catch(_){}if(entries.length)observer.callback(entries,observer)}};
Object.defineProperty(globalThis,'location',{value:{href:'about:blank'},writable:true,enumerable:true,configurable:true});
globalThis.__oryn_layout={styles:{},geometry:{},viewport:{},scroll:{},media:{}};
globalThis.__oryn_install_layout=value=>{globalThis.__oryn_layout=value;globalThis.innerWidth=Number(value.viewport?.width||0);globalThis.innerHeight=Number(value.viewport?.height||0)};
globalThis.__oryn_media={color_scheme:'light',reduced_motion:'no_preference'};
globalThis.matchMedia=query=>{query=String(query);const result={media:query,onchange:null,addEventListener(){},removeEventListener(){},addListener(){},removeListener(){}};Object.defineProperty(result,'matches',{get(){if(/prefers-color-scheme\s*:\s*dark/i.test(query))return __oryn_media.color_scheme==='dark';if(/prefers-color-scheme\s*:\s*light/i.test(query))return __oryn_media.color_scheme==='light';if(/prefers-reduced-motion\s*:\s*reduce/i.test(query))return __oryn_media.reduced_motion==='reduce';if(/prefers-reduced-motion\s*:\s*no-preference/i.test(query))return __oryn_media.reduced_motion==='no_preference';return true}});return result};
class OrynStorage {
  constructor(area,items=[]){this.area=area;this.fallback=new Map(items)}
  __response(operation,extra={}){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;return contextCall({operation:'storage',request:{operation,area:this.area,scope:{origin:location.origin,top_level_page_id:__oryn_page_id},...extra}}).response}
  get items(){const response=this.__response('entries');return response&&response.result==='entries'?new Map(response.entries):this.fallback}
  get length(){return this.items.size}
  key(index){return [...this.items.keys()][index]??null}
  getItem(key){key=String(key);const response=this.__response('get',{key});return response&&response.result==='value'?response.value??null:(this.fallback.has(key)?this.fallback.get(key):null)}
  setItem(key,value){key=String(key);value=String(value);const response=this.__response('set',{key,value});if(response===null)this.fallback.set(key,value);else __oryn_context_trace.push({kind:'storage_changed',detail:{area:this.area,key}})}
  removeItem(key){key=String(key);const response=this.__response('remove',{key});if(response===null)this.fallback.delete(key)}
  clear(){const response=this.__response('clear');if(response===null)this.fallback.clear()}
}
globalThis.__oryn_page_id=null;
globalThis.localStorage=new OrynStorage('local');
globalThis.sessionStorage=new OrynStorage('session');
globalThis.__oryn_restore_storage=(localItems,sessionItems)=>{
  globalThis.localStorage=new OrynStorage('local',localItems||[]);
  globalThis.sessionStorage=new OrynStorage('session',sessionItems||[]);
};
globalThis.setTimeout=(callback,delay=0,...args)=>{if(typeof callback!=='function')callback=Function(String(callback));return __oryn_timer_schedule(callback,delay,false,args)};
globalThis.setInterval=(callback,delay=0,...args)=>{if(typeof callback!=='function')callback=Function(String(callback));return __oryn_timer_schedule(callback,delay,true,args)};
globalThis.clearTimeout=id=>{__oryn_timer_cancel(Number(id))};
globalThis.clearInterval=globalThis.clearTimeout;
globalThis.__oryn_drain=()=>{if(typeof __oryn_poll_context==='function')__oryn_poll_context()};
class OrynURLSearchParams {
  constructor(init=''){this.items=[];for(const part of String(init).replace(/^\?/,'').split('&'))if(part){const [key,value='']=part.split('=');this.append(decodeURIComponent(key.replace(/\+/g,' ')),decodeURIComponent(value.replace(/\+/g,' ')))}}
  append(key,value){this.items.push([String(key),String(value)])}
  set(key,value){this.delete(key);this.append(key,value)}
  get(key){const item=this.items.find(item=>item[0]===String(key));return item?item[1]:null}
  getAll(key){return this.items.filter(item=>item[0]===String(key)).map(item=>item[1])}
  has(key){return this.items.some(item=>item[0]===String(key))}
  delete(key){this.items=this.items.filter(item=>item[0]!==String(key))}
  [Symbol.iterator](){return this.items[Symbol.iterator]()}
  toString(){return this.items.map(([key,value])=>encodeURIComponent(key)+'='+encodeURIComponent(value)).join('&')}
}
class OrynURL {
  constructor(input,base){input=String(input);if(base&&!/^[a-z][a-z0-9+.-]*:/i.test(input)){const parent=new OrynURL(base);const root=parent.host?parent.origin:parent.protocol;if(input.startsWith('#'))input=root+parent.pathname+parent.search+input;else if(input.startsWith('?'))input=root+parent.pathname+input;else input=input.startsWith('/')?root+input:root+parent.pathname.replace(/[^/]*$/,'')+input}const match=input.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i);if(!match)throw new TypeError('Invalid URL');this.protocol=match[1];this.hasAuthority=input.startsWith(this.protocol+'//');this.host=match[2]||'';this.hostname=this.host.split(':')[0];this.port=this.host.slice(this.hostname.length+1);this.pathname=match[3]||'/';this.search=match[4]||'';this.hash=match[5]||'';this.origin=this.host?this.protocol+'//'+this.host:'null';this.searchParams=new OrynURLSearchParams(this.search)}
  get href(){const query=this.searchParams.toString();return (this.host?this.origin:(this.hasAuthority?this.protocol+'//':this.protocol))+this.pathname+(query?'?'+query:'')+this.hash}
  set href(value){Object.assign(this,new OrynURL(value,this.href))}
  toString(){return this.href}
  toJSON(){return this.href}
}
globalThis.URL=OrynURL;globalThis.URLSearchParams=OrynURLSearchParams;
Object.defineProperty(globalThis,'location',{value:new OrynURL(location.href),writable:true,enumerable:true,configurable:true});
const contextCall=request=>JSON.parse(globalThis.__oryn_context_call(JSON.stringify(request)));
Object.defineProperty(Document.prototype,'cookie',{configurable:true,enumerable:true,get(){if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return '';const response=contextCall({operation:'get_document_cookie',page_id:__oryn_page_id});return response.result==='document_cookie'?response.value:''},set(value){if(typeof globalThis.__oryn_context_call==='function'&&__oryn_page_id!==null)contextCall({operation:'set_document_cookie',page_id:__oryn_page_id,value:String(value)})}});
Object.defineProperty(Document.prototype,'title',{configurable:true,enumerable:true,get(){const node=this.querySelector('title');return node?node.textContent:''},set(value){let node=this.querySelector('title');if(!node){node=this.createElement('title');const parent=this.querySelector('head')||this.documentElement;parent.appendChild(node)}node.textContent=String(value)}});
const __oryn_page_proxy=pageId=>pageId==null?null:({__orynPageId:pageId,__closed:false,get closed(){return this.__closed},focus(){contextCall({operation:'activate_page',source_page_id:__oryn_page_id,target_page_id:pageId})},close(){contextCall({operation:'close_page',source_page_id:__oryn_page_id,target_page_id:pageId});this.__closed=true},postMessage(data,targetOrigin='*'){contextCall({operation:'post_message',source_page_id:__oryn_page_id,target_page_id:pageId,target_origin:String(targetOrigin),data})}});
globalThis.__oryn_configure_context=(pageId,openerPageId)=>{__oryn_page_id=pageId;globalThis.opener=__oryn_page_proxy(openerPageId);if(typeof globalThis.__oryn_context_call==='function'){const response=contextCall({operation:'get_security_origin',page_id:pageId});if(response.result==='security_origin'){location.origin=response.serialized;globalThis.__oryn_opaque_origin=!!response.opaque}}};
globalThis.open=(url='')=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;const target=url?new OrynURL(url,location.href).href:null;const response=contextCall({operation:'open_page',opener_page_id:__oryn_page_id,url:target});if(response.result==='page_opened'){__oryn_context_trace.push({kind:'page_created',detail:{page_id:response.page_id,opener_page_id:__oryn_page_id}});return __oryn_page_proxy(response.page_id)}if(response.result==='page_denied')__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'popup.open',detail:response.reason}});return null};
const __oryn_global_listeners=new Map();const __oryn_websockets=new Set();let __oryn_context_event_id=0;
class SecurityPolicyViolationEvent extends Event {constructor(type,eventInitDict={}){super(type,{bubbles:true,cancelable:false});for(const [name,value] of Object.entries({documentURI:'',referrer:'',blockedURI:'',violatedDirective:'',effectiveDirective:'',originalPolicy:'',sourceFile:'',sample:'',disposition:'enforce',statusCode:0,lineNumber:0,columnNumber:0,...eventInitDict}))Object.defineProperty(this,name,{value,enumerable:true,writable:false,configurable:false})}}
globalThis.SecurityPolicyViolationEvent=SecurityPolicyViolationEvent;
globalThis.addEventListener=(type,callback)=>{const list=__oryn_global_listeners.get(String(type))||[];list.push(callback);__oryn_global_listeners.set(String(type),list)};
globalThis.removeEventListener=(type,callback)=>__oryn_global_listeners.set(String(type),(__oryn_global_listeners.get(String(type))||[]).filter(item=>item!==callback));
globalThis.dispatchEvent=event=>{for(const callback of __oryn_global_listeners.get(event.type)||[])callback.call(globalThis,event);const handler=globalThis['on'+event.type];if(typeof handler==='function')handler.call(globalThis,event);return !event.defaultPrevented};
globalThis.__oryn_poll_context=()=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return;const response=contextCall({operation:'poll_events',page_id:__oryn_page_id,after_event_id:__oryn_context_event_id,max_events:32});if(response.result==='events')for(const record of response.events||[]){__oryn_context_event_id=Math.max(__oryn_context_event_id,record.event_id);const item=record.event;if(!item)continue;if(item.kind==='storage'){const change=item.change||{};const event=new Event('storage',{});event.key=change.key??null;event.oldValue=change.old_value??null;event.newValue=change.new_value??null;event.storageArea=change.area==='local'?localStorage:sessionStorage;event.url=location.href;dispatchEvent(event)}else if(item.kind==='message'){const event=new Event('message',{});event.data=item.data;event.origin=item.source_origin||'';event.source=__oryn_page_proxy(item.source_page_id);dispatchEvent(event)}else if(item.kind==='csp_violation'){const value=item.violation||{};const event=new SecurityPolicyViolationEvent('securitypolicyviolation',{documentURI:value.document_uri||'',referrer:value.referrer||'',blockedURI:value.blocked_uri||'',violatedDirective:value.effective_directive||'',effectiveDirective:value.effective_directive||'',originalPolicy:value.original_policy||'',sourceFile:value.source_file||'',sample:value.sample||'',disposition:value.disposition||'enforce',statusCode:value.status_code||0,lineNumber:value.line_number||0,columnNumber:value.column_number||0});document.dispatchEvent(event);dispatchEvent(event)}else if(item.kind==='csp_report'){__oryn_context_trace.push({kind:'csp_report',detail:{endpoint:item.endpoint||'',format:item.format||'',outcome:item.outcome||''}})}}for(const socket of [...__oryn_websockets])socket.__poll()};
class OrynWebSocket {constructor(url,protocols=[]){this.url=new OrynURL(url,location.href).href;this.protocol='';this.binaryType='blob';this.readyState=0;this.__listeners=new Map();if(typeof protocols==='string')protocols=[protocols];const response=contextCall({operation:'web_socket_open',page_id:__oryn_page_id,url:this.url,protocols:[...protocols].map(String)});if(response.result!=='web_socket_opened')throw new Error('WebSocket open failed');this.__id=response.socket_id;this.protocol=response.protocol;__oryn_websockets.add(this);queueMicrotask(()=>{this.readyState=1;this.__event('open')})}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}send(data){const frame=typeof data==='string'?{kind:'text',value:data}:{kind:'binary',base64:bytesToBase64(bodyBytes(data))};const bytes=typeof data==='string'?new TextEncoder().encode(data).length:bodyBytes(data).length;contextCall({operation:'web_socket_send',page_id:__oryn_page_id,socket_id:this.__id,frame});__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'outbound',frame_type:frame.kind,bytes,url:this.url}})}close(code=1000,reason=''){contextCall({operation:'web_socket_close',page_id:__oryn_page_id,socket_id:this.__id,code,reason:String(reason)});this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code,reason:String(reason),wasClean:true})}__poll(){if(this.readyState!==1)return;const response=contextCall({operation:'web_socket_poll',page_id:__oryn_page_id,socket_id:this.__id,max_frames:32});if(response.result!=='web_socket_events')return;for(const frame of response.frames||[]){let bytes=0;if(frame.kind==='text'){bytes=new TextEncoder().encode(frame.value).length;this.__event('message',{data:frame.value})}else if(frame.kind==='binary'){const value=base64ToBytes(frame.base64);bytes=value.length;this.__event('message',{data:this.binaryType==='arraybuffer'?value.buffer:new Blob([value])})}else if(frame.kind==='close'){this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code:frame.code??1005,reason:frame.reason||'',wasClean:true})}__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'inbound',frame_type:frame.kind,bytes,url:this.url}})}}}
Object.assign(OrynWebSocket,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});Object.assign(OrynWebSocket.prototype,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});globalThis.WebSocket=OrynWebSocket;
const __oryn_workers=new Set();
const __oryn_worker_clone=value=>{const encoded=JSON.stringify(value);if(encoded===undefined){const error=new TypeError('dedicated-worker messages must be structured-cloneable JSON values');error.name='DataCloneError';throw error}return encoded};
class OrynWorker {constructor(url,options={}){this.onmessage=null;this.onerror=null;this.__listeners=new Map();this.__closed=false;const target=new OrynURL(url,location.href).href;const response=contextCall({operation:'load_worker_script',page_id:__oryn_page_id,url:target,module:String(options.type||'classic')==='module'});if(response.result!=='worker_script')throw new Error('Worker script load failed');try{this.__id=__oryn_worker_create(response.source,response.url,!!response.module);__oryn_workers.add(this);queueMicrotask(()=>this.__poll())}catch(error){this.__closed=true;queueMicrotask(()=>this.__event('error',{error,message:String(error)}))}}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}__poll(){if(this.__closed||!this.__id)return;for(const encoded of JSON.parse(__oryn_worker_take(this.__id))){this.__event('message',{data:JSON.parse(encoded)})}}postMessage(data){if(this.__closed||!this.__id)return;__oryn_worker_post(this.__id,__oryn_worker_clone(data));this.__poll()}terminate(){if(this.__id)__oryn_worker_terminate(this.__id);this.__closed=true;this.__id=0;__oryn_workers.delete(this)}}
globalThis.Worker=OrynWorker;
const __oryn_context_poll_without_workers=globalThis.__oryn_poll_context;globalThis.__oryn_poll_context=()=>{__oryn_context_poll_without_workers();for(const worker of [...__oryn_workers])worker.__poll()};
class OrynTextEncoder {encode(value=''){const encoded=unescape(encodeURIComponent(String(value)));return Uint8Array.from(encoded,char=>char.charCodeAt(0))}}
class OrynTextDecoder {decode(value=new Uint8Array()){let encoded='';for(const byte of new Uint8Array(value.buffer||value,value.byteOffset||0,value.byteLength??value.length))encoded+=String.fromCharCode(byte);return decodeURIComponent(escape(encoded))}}
globalThis.TextEncoder=OrynTextEncoder;globalThis.TextDecoder=OrynTextDecoder;
const b64chars='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const bytesToBase64=value=>{const bytes=value instanceof Uint8Array?value:new Uint8Array(value);let out='';for(let index=0;index<bytes.length;index+=3){const first=bytes[index],second=bytes[index+1],third=bytes[index+2];out+=b64chars[first>>2];out+=b64chars[((first&3)<<4)|((second??0)>>4)];out+=second===undefined?'=':b64chars[((second&15)<<2)|((third??0)>>6)];out+=third===undefined?'=':b64chars[third&63]}return out};
const base64ToBytes=value=>{value=String(value).replace(/\s/g,'');const out=[];for(let index=0;index<value.length;index+=4){const first=b64chars.indexOf(value[index]),second=b64chars.indexOf(value[index+1]),third=value[index+2]==='='?-1:b64chars.indexOf(value[index+2]),fourth=value[index+3]==='='?-1:b64chars.indexOf(value[index+3]);out.push((first<<2)|(second>>4));if(third>=0)out.push(((second&15)<<4)|(third>>2));if(fourth>=0)out.push(((third&3)<<6)|fourth)}return Uint8Array.from(out)};
globalThis.btoa=value=>bytesToBase64(Uint8Array.from(String(value),char=>char.charCodeAt(0)&255));globalThis.atob=value=>{let result='';for(const byte of base64ToBytes(value))result+=String.fromCharCode(byte);return result};
const bodyBytes=body=>{if(body==null)return new Uint8Array();if(body instanceof OrynBlob)return body.__bytes.slice();if(body instanceof ArrayBuffer)return new Uint8Array(body.slice(0));if(ArrayBuffer.isView(body))return new Uint8Array(body.buffer.slice(body.byteOffset,body.byteOffset+body.byteLength));return new TextEncoder().encode(String(body))};
class OrynBlob {constructor(parts=[],options={}){const chunks=parts.map(bodyBytes),length=chunks.reduce((sum,item)=>sum+item.length,0);this.__bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){this.__bytes.set(chunk,offset);offset+=chunk.length}this.type=String(options.type||'').toLowerCase()}get size(){return this.__bytes.length}slice(start=0,end=this.size,type=''){return new OrynBlob([this.__bytes.slice(start,end)],{type})}arrayBuffer(){return Promise.resolve(this.__bytes.buffer.slice(0))}text(){return Promise.resolve(new TextDecoder().decode(this.__bytes))}stream(){return new OrynReadableStream({start:controller=>{controller.enqueue(this.__bytes.slice());controller.close()}})}}
class OrynFile extends OrynBlob {constructor(parts,name,options={}){super(parts,options);this.name=String(name);this.lastModified=Number(options.lastModified??0);this.webkitRelativePath=''}}
globalThis.Blob=OrynBlob;globalThis.File=OrynFile;
class OrynReadableStream {constructor(source={}){this.__queue=[];this.__closed=false;this.__waiting=[];this.__cancel=source.cancel;this.locked=false;const controller={enqueue:value=>{const waiter=this.__waiting.shift();waiter?waiter({done:false,value}):this.__queue.push(value)},close:()=>{this.__closed=true;for(const waiter of this.__waiting.splice(0))waiter({done:true})}};if(source.start)source.start(controller);this.__pull=source.pull?()=>source.pull(controller):null}getReader(){this.locked=true;const stream=this;return {read(){if(stream.__queue.length)return Promise.resolve({done:false,value:stream.__queue.shift()});if(stream.__closed)return Promise.resolve({done:true});return new Promise(resolve=>{stream.__waiting.push(resolve);if(stream.__pull)stream.__pull()})},cancel:reason=>stream.cancel(reason),releaseLock(){stream.locked=false}}}cancel(reason){this.__closed=true;for(const waiter of this.__waiting.splice(0))waiter({done:true});return Promise.resolve(this.__cancel?this.__cancel(reason):undefined)}pipeTo(destination){const reader=this.getReader(),writer=destination.getWriter();const pump=()=>reader.read().then(item=>item.done?writer.close():writer.write(item.value).then(pump));return pump()}pipeThrough(transform){this.pipeTo(transform.writable);return transform.readable}}
class OrynWritableStream {constructor(sink={}){this.__sink=sink;this.locked=false}getWriter(){this.locked=true;const stream=this;return {write:chunk=>Promise.resolve(stream.__sink.write?stream.__sink.write(chunk):undefined),close:()=>Promise.resolve(stream.__sink.close?stream.__sink.close():undefined),abort:reason=>Promise.resolve(stream.__sink.abort?stream.__sink.abort(reason):undefined),get ready(){return Promise.resolve()},get desiredSize(){return 1},releaseLock(){stream.locked=false}}}}
class OrynTransformStream {constructor(transformer={}){let controller;this.readable=new OrynReadableStream({start:value=>controller=value});this.writable=new OrynWritableStream({write:chunk=>transformer.transform?transformer.transform(chunk,controller):controller.enqueue(chunk),close:()=>{if(transformer.flush)return Promise.resolve(transformer.flush(controller)).then(()=>controller.close());controller.close()}})}}
globalThis.ReadableStream=OrynReadableStream;globalThis.WritableStream=OrynWritableStream;globalThis.TransformStream=OrynTransformStream;
class OrynAbortSignal {constructor(){this.aborted=false;this.reason=undefined;this.__listeners=[]}addEventListener(type,callback){if(type==='abort')this.__listeners.push(callback)}removeEventListener(type,callback){if(type==='abort')this.__listeners=this.__listeners.filter(item=>item!==callback)}throwIfAborted(){if(this.aborted)throw this.reason}}
class OrynAbortController {constructor(){this.signal=new OrynAbortSignal()}abort(reason=new Error('The operation was aborted')){if(this.signal.aborted)return;this.signal.aborted=true;this.signal.reason=reason;for(const callback of this.signal.__listeners.splice(0))callback.call(this.signal,new Event('abort'))}}
globalThis.AbortSignal=OrynAbortSignal;globalThis.AbortController=OrynAbortController;
class OrynHeaders {
  constructor(init=[]){this.items=[];const entries=init instanceof OrynHeaders?[...init]:(Array.isArray(init)||(init&&init[Symbol.iterator])?[...init]:Object.entries(init||{}));for(const [name,value] of entries)this.append(name,value)}
  set(name,value){name=String(name).toLowerCase();this.delete(name);this.items.push([name,String(value)])}
  append(name,value){name=String(name).toLowerCase();const old=this.get(name);this.set(name,old===null?value:`${old}, ${value}`)}
  get(name){name=String(name).toLowerCase();const item=this.items.find(([key])=>key===name);return item?item[1]:null}
  has(name){return this.get(name)!==null}
  delete(name){name=String(name).toLowerCase();this.items=this.items.filter(([key])=>key!==name)}
  [Symbol.iterator](){return this.items[Symbol.iterator]()}
}
class OrynResponse {
  constructor(body=null,{status=200,url='',headers=[],bodyChunks=[],streamId=null}={}){this.__bytes=streamId===null?bodyBytes(body):null;this.__chunks=bodyChunks.length?bodyChunks.map(bodyBytes):(this.__bytes?[this.__bytes.slice()]:[]);this.__streamId=streamId;this.__consumePromise=null;this.status=status;this.url=url;this.headers=new OrynHeaders(headers);let reading=false;this.body=new OrynReadableStream({start:controller=>{for(const chunk of this.__chunks)controller.enqueue(chunk.slice());if(streamId===null)controller.close()},pull:streamId===null?null:async controller=>{if(reading)return;reading=true;try{const result=JSON.parse(await globalThis.__oryn_stream_read_async(streamId));const chunk=base64ToBytes(result.body_base64||'');if(chunk.length){controller.enqueue(chunk);__oryn_context_trace.push({kind:'stream_chunk',detail:{direction:'response',bytes:chunk.length}})}if(result.done)controller.close()}finally{reading=false}},cancel:streamId===null?null:()=>{contextCall({operation:'http_stream_abort',page_id:__oryn_page_id,stream_id:streamId})}})}
  get ok(){return this.status>=200&&this.status<300}
  __consume(){if(this.__bytes)return Promise.resolve(this.__bytes);if(this.__consumePromise)return this.__consumePromise;const reader=this.body.getReader(),chunks=[];let length=0;this.__consumePromise=(async()=>{for(;;){const item=await reader.read();if(item.done)break;const chunk=bodyBytes(item.value);chunks.push(chunk);length+=chunk.length}const bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length}this.__bytes=bytes;return bytes})();return this.__consumePromise}
  text(){return this.__consume().then(bytes=>new TextDecoder().decode(bytes))}
  json(){return this.text().then(JSON.parse)}
  arrayBuffer(){return this.__consume().then(bytes=>bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength))}
  blob(){return this.__consume().then(bytes=>new OrynBlob([bytes],{type:this.headers.get('content-type')||''}))}
  static json(value,init={}){const headers=new OrynHeaders(init.headers||[]);if(!headers.has('content-type'))headers.set('content-type','application/json');return new OrynResponse(JSON.stringify(value),{...init,headers})}
}
globalThis.Headers=OrynHeaders;
globalThis.Response=OrynResponse;
class OrynRequest {constructor(input,init={}){this.url=new OrynURL(input&&input.url?input.url:String(input),location.href).href;this.method=String(init.method||'GET').toUpperCase();this.headers=new OrynHeaders(init.headers||[]);this.body=init.body??null;this.signal=init.signal||new OrynAbortSignal();this.credentials=init.credentials||'same-origin'}}
globalThis.Request=OrynRequest;
const __orynInputFiles=node=>{const encoded=node.getAttribute('data-oryn-upload-files');if(!encoded)return [];try{return JSON.parse(encoded).map(file=>new OrynFile([base64ToBytes(file.body_base64)],file.filename,{type:file.media_type,lastModified:file.last_modified}))}catch(_){return []}};
if(globalThis.HTMLInputElement)Object.defineProperty(HTMLInputElement.prototype,'files',{configurable:true,get(){return __orynInputFiles(this)}});
class OrynFormData {constructor(form){this.items=[];if(form)for(const node of [...form.querySelectorAll('input'),...form.querySelectorAll('textarea'),...form.querySelectorAll('select')]){const name=node.getAttribute('name');if(!name||node.disabled)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;if(type==='file'){for(const file of __orynInputFiles(node))this.append(name,file);continue}this.append(name,node.value)}}append(name,value,filename){if(filename!==undefined&&value instanceof OrynBlob)value=new OrynFile([value.__bytes],filename,{type:value.type});this.items.push([String(name),value instanceof OrynBlob?value:String(value)])}set(name,value,filename){this.delete(name);this.append(name,value,filename)}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1])}has(name){return this.items.some(item=>item[0]===String(name))}delete(name){this.items=this.items.filter(item=>item[0]!==String(name))}entries(){return this.items[Symbol.iterator]()}keys(){return this.items.map(item=>item[0])[Symbol.iterator]()}values(){return this.items.map(item=>item[1])[Symbol.iterator]()}[Symbol.iterator](){return this.entries()}}
globalThis.FormData=OrynFormData;
globalThis.crypto={getRandomValues(array){const encoded=__oryn_crypto_sync('random',String(array.byteLength));array.set(base64ToBytes(encoded));return array},randomUUID(){const bytes=new Uint8Array(16);this.getRandomValues(bytes);bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`},subtle:{digest(algorithm,value){if(String(algorithm).toUpperCase()!=='SHA-256')return Promise.reject(new Error('unsupported digest'));const result=base64ToBytes(__oryn_crypto_sync('sha256',bytesToBase64(bodyBytes(value))));return Promise.resolve(result.buffer)}}};
const __oryn_idb_call=command=>{const outer=contextCall({operation:'storage',request:{operation:'indexed_db',origin:location.origin,command}});if(outer.result!=='storage'||outer.response.result!=='indexed_db')throw new Error('invalid IndexedDB response');return outer.response.response};
const __oryn_clone_encode=value=>{if(value instanceof ArrayBuffer)return {__oryn_clone:'array_buffer',base64:bytesToBase64(value)};if(ArrayBuffer.isView(value))return {__oryn_clone:'typed_array',type:value.constructor.name,base64:bytesToBase64(new Uint8Array(value.buffer,value.byteOffset,value.byteLength))};if(value instanceof OrynFile)return {__oryn_clone:'file',base64:bytesToBase64(value.__bytes),type:value.type,name:value.name,last_modified:value.lastModified};if(value instanceof OrynBlob)return {__oryn_clone:'blob',base64:bytesToBase64(value.__bytes),type:value.type};if(Array.isArray(value))return value.map(__oryn_clone_encode);if(value&&typeof value==='object'){const result={};for(const [key,item] of Object.entries(value))result[key]=__oryn_clone_encode(item);return result}return value};
const __oryn_clone_decode=value=>{if(Array.isArray(value))return value.map(__oryn_clone_decode);if(value&&typeof value==='object'){if(value.__oryn_clone==='array_buffer')return base64ToBytes(value.base64).buffer;if(value.__oryn_clone==='typed_array'){const bytes=base64ToBytes(value.base64),ctor=globalThis[value.type];return new ctor(bytes.buffer)}if(value.__oryn_clone==='blob')return new Blob([base64ToBytes(value.base64)],{type:value.type});if(value.__oryn_clone==='file')return new File([base64ToBytes(value.base64)],value.name,{type:value.type,lastModified:value.last_modified});const result={};for(const [key,item] of Object.entries(value))result[key]=__oryn_clone_decode(item);return result}return value};
class OrynIDBRequest {constructor(){this.result=undefined;this.error=null;this.onsuccess=null;this.onerror=null;this.onupgradeneeded=null}__fire(type,extra={}){const event={type,target:this,...extra};const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}__success(value){queueMicrotask(()=>{this.result=value;this.__fire('success')})}__failure(error){queueMicrotask(()=>{this.error=error;this.__fire('error')})}}
class OrynIDBIndex {constructor(store,name){this.store=store;this.name=String(name)}get(query){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'cursor',database:this.store.transaction.db.name,store:this.store.name,index:this.name,query,reverse:false});request.__success(__oryn_clone_decode(response.records[0]?.value))}catch(error){request.__failure(error)}return request}}
class OrynIDBObjectStore {constructor(transaction,name){this.transaction=transaction;this.name=String(name)}put(value,key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:__oryn_clone_encode(value),add_only:false}]});request.__success(response.key)}catch(error){request.__failure(error)}return request}add(value,key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:__oryn_clone_encode(value),add_only:true}]});request.__success(response.key)}catch(error){request.__failure(error)}return request}get(key){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'get',database:this.transaction.db.name,store:this.name,key});request.__success(__oryn_clone_decode(response.value))}catch(error){request.__failure(error)}return request}createIndex(name,keyPath,options={}){__oryn_idb_call({command:'create_index',database:this.transaction.db.name,store:this.name,definition:{name:String(name),key_path:String(keyPath),unique:!!options.unique}});return new OrynIDBIndex(this,name)}index(name){return new OrynIDBIndex(this,name)}}
class OrynIDBTransaction {constructor(db,names,mode='readonly'){this.db=db;this.mode=mode;this.names=Array.isArray(names)?names:[String(names)]}objectStore(name){return new OrynIDBObjectStore(this,name)}}
class OrynIDBDatabase {constructor(name,version,stores=[]){this.name=String(name);this.version=version;this.objectStoreNames=stores}createObjectStore(name,options={}){const response=__oryn_idb_call({command:'create_object_store',database:this.name,definition:{name:String(name),key_path:options.keyPath??null,auto_increment:!!options.autoIncrement}});this.objectStoreNames=response.object_stores;return new OrynIDBObjectStore(new OrynIDBTransaction(this,[name],'versionchange'),name)}transaction(names,mode='readonly'){return new OrynIDBTransaction(this,names,mode)}close(){}}
globalThis.indexedDB={open(name,version=0){const request=new OrynIDBRequest();try{const response=__oryn_idb_call({command:'open',name:String(name),version:Number(version)||0});const db=new OrynIDBDatabase(name,response.version,response.object_stores);queueMicrotask(()=>{request.result=db;if(response.upgrade_needed)request.__fire('upgradeneeded',{oldVersion:response.old_version,newVersion:response.version});request.__fire('success')})}catch(error){request.__failure(error)}return request},deleteDatabase(name){const request=new OrynIDBRequest();try{__oryn_idb_call({command:'delete_database',name:String(name)});request.__success(undefined)}catch(error){request.__failure(error)}return request}};
if(typeof globalThis.__oryn_fetch_async==='function')globalThis.fetch=(input,init={})=>{
  const request=input instanceof OrynRequest?new OrynRequest(input.url,{method:init.method||input.method,headers:init.headers||input.headers,body:init.body??input.body,signal:init.signal||input.signal,credentials:init.credentials||input.credentials}):new OrynRequest(input,init);
  const method=request.method,headers=[...request.headers],signal=request.signal||null;if(signal&&signal.aborted)return Promise.reject(signal.reason||new Error('AbortError'));
  const bytes=bodyBytes(request.body);
  __oryn_context_trace.push({kind:'request_started',detail:{method,url:request.url,bytes:bytes.length}});
  return globalThis.__oryn_fetch_async(request.url,method,bytesToBase64(bytes),JSON.stringify(headers),'base64',request.credentials).then(raw=>{
    if(signal&&signal.aborted)throw signal.reason||new Error('AbortError');
    const result=JSON.parse(raw);let encoded=atob(result.body_base64||'');const responseBytes=Uint8Array.from(encoded,value=>value.charCodeAt(0));
    const chunks=(result.body_chunks_base64||[]).map(base64ToBytes);__oryn_context_trace.push({kind:'response_received',detail:{url:result.url,status:result.status,bytes:result.stream_id===null?responseBytes.length:null}});for(const chunk of chunks)__oryn_context_trace.push({kind:'stream_chunk',detail:{direction:'response',bytes:chunk.length}});
    const response=new OrynResponse(responseBytes,{status:result.status,url:result.url,headers:result.headers,bodyChunks:chunks,streamId:result.stream_id});if(signal)signal.addEventListener('abort',()=>response.body.cancel(signal.reason));return response;
  });
};
class OrynXMLHttpRequest {
  constructor(){this.readyState=0;this.status=0;this.responseType='';this.response=null;this.responseText='';this.responseURL='';this.timeout=0;this.withCredentials=false;this.__headers=new OrynHeaders();this.__responseHeaders=new OrynHeaders();this.__listeners=new Map();this.__aborted=false;this.__timedOut=false}
  addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}
  removeEventListener(type,callback){this.__listeners.set(String(type),(this.__listeners.get(String(type))||[]).filter(item=>item!==callback))}
  __event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}
  __state(value){this.readyState=value;this.__event('readystatechange')}
  open(method,url,async=true){if(async===false){__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'xhr.synchronous_main_thread',detail:'synchronous main-thread XHR is unsupported'}});throw new Error('synchronous main-thread XHR is unsupported')}this.__method=String(method).toUpperCase();this.__url=new OrynURL(url,location.href).href;this.__state(1)}
  setRequestHeader(name,value){this.__headers.append(name,value)}
  getResponseHeader(name){return this.__responseHeaders.get(name)}
  getAllResponseHeaders(){return [...this.__responseHeaders].map(([name,value])=>`${name}: ${value}\r\n`).join('')}
  send(body=null){if(this.readyState!==1)throw new Error('InvalidStateError');this.__event('loadstart');const controller=new AbortController();this.__controller=controller;const started=Date.now();const timeout=()=>{if(this.__aborted||this.__timedOut)return;this.__timedOut=true;controller.abort(new Error('TimeoutError'));this.status=0;this.__event('timeout');this.__event('loadend')};if(this.timeout>0)this.__timer=setTimeout(timeout,this.timeout);Promise.resolve().then(()=>{if(this.__aborted)return null;return fetch(this.__url,{method:this.__method,headers:this.__headers,body,credentials:this.withCredentials?'include':'same-origin',signal:controller.signal})}).then(async result=>{if(!result||this.__aborted||this.__timedOut)return;if(this.timeout>0&&Date.now()-started>=this.timeout){timeout();return}if(this.__timer)clearTimeout(this.__timer);this.status=result.status;this.responseURL=result.url;this.__responseHeaders=result.headers;this.__state(2);this.__state(3);const bytes=new Uint8Array(await result.arrayBuffer());if(this.__aborted||this.__timedOut)return;this.responseText=new TextDecoder().decode(bytes);this.response=this.responseType==='arraybuffer'?bytes.buffer.slice(0):this.responseType==='blob'?new Blob([bytes],{type:this.__responseHeaders.get('content-type')||''}):this.responseType==='json'?JSON.parse(this.responseText||'null'):this.responseText;this.__event('progress',{lengthComputable:true,loaded:bytes.length,total:bytes.length});this.__state(4);this.__event('load');this.__event('loadend')}).catch(error=>{if(this.__aborted||this.__timedOut)return;this.status=0;this.__event('error',{error});this.__event('loadend')})}
  abort(){if(this.__aborted)return;this.__aborted=true;if(this.__controller)this.__controller.abort(new Error('AbortError'));this.status=0;this.readyState=0;this.__event('abort');this.__event('loadend')}
}
Object.assign(OrynXMLHttpRequest,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});Object.assign(OrynXMLHttpRequest.prototype,{UNSENT:0,OPENED:1,HEADERS_RECEIVED:2,LOADING:3,DONE:4});globalThis.XMLHttpRequest=OrynXMLHttpRequest;
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextTraceEntry {
    pub kind: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

pub struct RustExecutableDocument {
    pub document: ParsedDocument,
    pub diagnostics: Vec<oryn_common::v3::CapabilityDiagnostic>,
    live_dom: Rc<RefCell<DomRuntime>>,
    host: RawV8Host,
    top_realm: RealmId,
    node_realms: BTreeMap<NodeId, RealmId>,
    base_url: Option<Url>,
    resources: BTreeMap<String, String>,
    broker: Option<SharedNetworkBroker>,
    next_dynamic_request_id: u64,
    #[cfg(unix)]
    context: Option<(
        crate::context_service::SharedContextClient,
        oryn_common::v3::PageId,
        Option<oryn_common::v3::PageId>,
    )>,
}

impl RustExecutableDocument {
    pub fn load(html_source: &str) -> Result<Self, HostError> {
        Self::load_with_resources(html_source, &BTreeMap::new())
    }

    pub fn load_with_resources(
        html_source: &str,
        resources: &BTreeMap<String, String>,
    ) -> Result<Self, HostError> {
        Self::load_with_environment(html_source, resources, None, None)
    }

    pub fn load_with_environment(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
    ) -> Result<Self, HostError> {
        Self::load_with_limits(html_source, resources, broker, base_url, 256 * 1024 * 1024)
    }

    pub fn load_with_limits(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            html_source,
            resources,
            broker,
            base_url,
            max_heap_bytes,
            #[cfg(unix)]
            None,
        )
    }

    #[cfg(unix)]
    pub(crate) fn load_with_context_and_limits(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        context: (
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        ),
    ) -> Result<Self, HostError> {
        Self::load_with_options(
            html_source,
            resources,
            broker,
            base_url,
            max_heap_bytes,
            Some(context),
        )
    }

    fn load_with_options(
        html_source: &str,
        resources: &BTreeMap<String, String>,
        broker: Option<SharedNetworkBroker>,
        base_url: Option<Url>,
        max_heap_bytes: usize,
        #[cfg(unix)] context: Option<(
            crate::context_service::SharedContextClient,
            oryn_common::v3::PageId,
            Option<oryn_common::v3::PageId>,
        )>,
    ) -> Result<Self, HostError> {
        #[cfg(unix)]
        let context_identity = context
            .as_ref()
            .map(|(_, page_id, opener_page_id)| (*page_id, *opener_page_id));
        #[cfg(unix)]
        let context_handle = context.clone();
        let broker_handle = broker.clone();
        let parsed = html::parse_streamed(html_source, 4 * 1024);
        let scripts = parsed
            .dom
            .iter()
            .filter(|(_, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "script")
            })
            .map(|(id, node)| {
                let script_type = node.attributes.get("type").cloned();
                let parser_blocking = script_type.as_deref().is_none_or(|kind| {
                    matches!(
                        kind,
                        "" | "text/javascript" | "application/javascript" | "text/babel"
                    )
                }) && !node.attributes.contains_key("async")
                    && !node.attributes.contains_key("defer");
                (
                    id,
                    node.attributes.get("src").cloned(),
                    script_type,
                    parsed.dom.text_content(id).unwrap_or_default(),
                    parser_blocking,
                )
            })
            .collect::<Vec<_>>();
        let frame_sources = parsed
            .dom
            .iter()
            .filter(|(_, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "iframe")
            })
            .map(|(id, node)| (id, node.attributes.get("src").cloned()))
            .collect::<Vec<_>>();
        let quirks_mode = parsed.quirks_mode;
        let parser_events = parsed.events.clone();
        let live_dom = Rc::new(RefCell::new(parsed.into_runtime().map_err(|error| {
            HostError::Execution(format!("failed to initialize Rust DOM: {error}"))
        })?));
        #[cfg(unix)]
        let mut host = match context {
            Some((client, page_id, _opener_page_id)) => {
                RawV8Host::new_with_network_context_and_heap_limit(
                    broker.clone(),
                    base_url.clone(),
                    client,
                    page_id,
                    max_heap_bytes,
                )
            }
            None => match (broker, base_url.clone()) {
                (Some(broker), Some(base_url)) => {
                    RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
                }
                _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
            },
        };
        #[cfg(not(unix))]
        let mut host = match (broker, base_url.clone()) {
            (Some(broker), Some(base_url)) => {
                RawV8Host::new_with_network_and_heap_limit(broker, base_url, max_heap_bytes)
            }
            _ => RawV8Host::new_with_heap_limit(max_heap_bytes),
        };
        host.install_dom(live_dom.clone(), 1)?;
        if let Some(config) = resources.get("oryn:trusted-types") {
            let config = serde_json::from_str(config).map_err(|error| {
                HostError::Execution(format!("invalid Trusted Types policy snapshot: {error}"))
            })?;
            let allow_eval = resources
                .get("oryn:csp-allow-eval")
                .is_none_or(|value| value == "true");
            host.configure_trusted_types(config, allow_eval)?;
        }
        let top_realm = host.active_realm();
        host.execute(RUST_RUNTIME_GLOBALS)?;
        #[cfg(unix)]
        if let Some((page_id, opener_page_id)) = context_identity {
            host.execute(&format!(
                "__oryn_configure_context({}, {})",
                page_id.0,
                opener_page_id
                    .map(|page_id| page_id.0.to_string())
                    .unwrap_or_else(|| "null".into())
            ))?;
        }
        host.set_module_sources(resources);
        if let Some(base_url) = &base_url {
            let url = serde_json::to_string(base_url.as_str())
                .map_err(|error| HostError::Execution(error.to_string()))?;
            // Seed the realm's committed URL without invoking Location's public
            // navigation path. Internal fixture URLs are valid document origins
            // but must never escape through the network broker as navigations.
            host.execute(&format!(
                "Object.assign(globalThis.location,new OrynURL({url}))"
            ))?;
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &context_handle {
                client
                    .request(crate::context_service::ContextServiceRequest::CommitPage {
                        page_id: *page_id,
                        url: base_url.to_string(),
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
        }
        let local_storage = resources
            .get("oryn:local-storage")
            .map(String::as_str)
            .unwrap_or("[]");
        let session_storage = resources
            .get("oryn:session-storage")
            .map(String::as_str)
            .unwrap_or("[]");
        let local_storage = serde_json::to_string(local_storage)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let session_storage = serde_json::to_string(session_storage)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        host.execute(&format!(
            "__oryn_restore_storage(JSON.parse({local_storage}),JSON.parse({session_storage}))"
        ))?;
        let mut diagnostics = Vec::new();
        let mut node_realms = BTreeMap::new();
        for (frame_index, (frame, source)) in frame_sources.into_iter().enumerate() {
            let Some(source) = source else { continue };
            let frame_url = base_url
                .as_ref()
                .and_then(|base| base.join(&source).ok())
                .or_else(|| Url::parse(&source).ok());
            let same_origin = base_url
                .as_ref()
                .zip(frame_url.as_ref())
                .is_some_and(|(parent, child)| parent.origin() == child.origin());
            let Some(frame_html) = resources.get(&source).filter(|_| same_origin) else {
                diagnostics.push(oryn_common::v3::CapabilityDiagnostic {
                    capability: "html.frame.cross_origin".into(),
                    support: oryn_common::v3::SupportLevel::Unsupported,
                    alternatives: vec![oryn_common::v3::ExecutionDomain::Chromium],
                    handoff_lossy: true,
                    detail: Some(format!("frame {source:?} is opaque in the native domain")),
                });
                continue;
            };
            let child = html::parse_streamed(frame_html, 4 * 1024);
            let child_scripts = child
                .dom
                .iter()
                .filter(|(_, node)| {
                    matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "script")
                })
                .map(|(id, _)| child.dom.text_content(id).unwrap_or_default())
                .collect::<Vec<_>>();
            let child_document = live_dom
                .borrow_mut()
                .import_subtree(&child.dom, child.document)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            live_dom
                .borrow_mut()
                .append_child(frame, child_document)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let realm = host.create_realm_with_dom_root(
                frame_url
                    .as_ref()
                    .map(|url| url.origin().ascii_serialization())
                    .unwrap_or_else(|| "null".into()),
                1,
                Some(frame_index as u64 + 1),
                live_dom.clone(),
                child_document,
            )?;
            for node in subtree_nodes(&live_dom.borrow(), child_document)? {
                node_realms.insert(node, realm);
            }
            host.activate_realm(realm)?;
            host.execute(RUST_RUNTIME_GLOBALS)?;
            host.set_module_sources(resources);
            if let Some(frame_url) = &frame_url {
                let url = serde_json::to_string(frame_url.as_str())
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                // Frame creation installs an already-authorized document URL;
                // assigning href here would incorrectly schedule navigation.
                host.execute(&format!(
                    "Object.assign(globalThis.location,new OrynURL({url}))"
                ))?;
            }
            #[cfg(unix)]
            if let Some((page_id, _)) = context_identity {
                host.execute(&format!("__oryn_configure_context({}, null)", page_id.0))?;
            }
            let imported_scripts = live_dom
                .borrow()
                .query_selector_all(child_document, "script")
                .unwrap_or_default();
            for (script, source) in imported_scripts.into_iter().zip(child_scripts) {
                host.set_document_write_anchor(Some(script))?;
                if let Err(error) = host.execute(&source) {
                    diagnostics.push(script_diagnostic(error));
                }
            }
            host.set_document_write_anchor(None)?;
            host.activate_realm(top_realm)?;
            host.set_frame_document(frame, child_document)?;
        }
        for (script, src, script_type, inline, parser_blocking) in scripts {
            let parser_suspension = parser_blocking
                .then(|| live_dom.borrow_mut().suspend_parser_tail(script))
                .transpose()
                .map_err(|error| HostError::Execution(error.to_string()))?;
            host.set_document_write_anchor(Some(script))?;
            if script_type.as_deref() == Some("module") {
                let name = src.as_deref().unwrap_or("oryn:inline-module");
                let source = src
                    .as_ref()
                    .and_then(|src| resources.get(src))
                    .map(String::as_str)
                    .unwrap_or(&inline);
                if let Err(error) = host.execute_module(name, source, resources) {
                    diagnostics.push(script_diagnostic(error));
                }
                if let Some(suspension) = parser_suspension {
                    live_dom
                        .borrow_mut()
                        .resume_parser_tail(suspension)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                continue;
            }
            if script_type.as_deref().is_some_and(|kind| {
                !matches!(
                    kind,
                    "" | "text/javascript" | "application/javascript" | "text/babel"
                )
            }) {
                if let Some(suspension) = parser_suspension {
                    live_dom
                        .borrow_mut()
                        .resume_parser_tail(suspension)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                continue;
            }
            let source = src
                .as_ref()
                .and_then(|src| resources.get(src))
                .map(String::as_str)
                .unwrap_or(&inline);
            let transformed;
            let source = if script_type.as_deref() == Some("text/babel") {
                let encoded = serde_json::to_string(source)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                transformed = host.evaluate_string(&format!(
                    "Babel.transform({encoded},{{presets:['react']}}).code"
                ))?;
                transformed.as_str()
            } else {
                source
            };
            if let Err(error) = host.execute(source) {
                diagnostics.push(script_diagnostic(error));
            }
            if let Some(suspension) = parser_suspension {
                live_dom
                    .borrow_mut()
                    .resume_parser_tail(suspension)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
        }
        host.set_document_write_anchor(None)?;
        host.drain_async_fetches(true)?;
        let (dom, document) = live_dom.borrow().snapshot();
        let mut page = Self {
            document: ParsedDocument {
                dom,
                document,
                errors: Vec::new(),
                quirks_mode,
                events: parser_events,
            },
            diagnostics,
            live_dom,
            host,
            top_realm,
            node_realms,
            base_url,
            resources: resources.clone(),
            broker: broker_handle,
            next_dynamic_request_id: 1,
            #[cfg(unix)]
            context: context_handle,
        };
        // Dynamic external/module scripts inserted by parser-time JavaScript
        // are force-async but still participate in the document load barrier.
        page.drain_dynamic_scripts()?;
        page.host.drain_async_fetches(true)?;
        // Zero-delay framework commits queued before load run at this
        // checkpoint. Future timers retain their deadlines and must not be
        // pulled forward merely because navigation reached the load barrier.
        page.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        page.host.execute(
            "document.dispatchEvent(new Event('DOMContentLoaded',{bubbles:true}));dispatchEvent(new Event('load'))",
        )?;
        page.host.drain_scheduled_timers(false)?;
        page.checkpoint()?;
        Ok(page)
    }

    pub fn execute(&mut self, source: &str) -> Result<(), HostError> {
        self.host.execute(source)?;
        self.checkpoint()
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, HostError> {
        if source.contains("__oryn_drain(true)") {
            self.drain_tasks()?;
        }
        let value = self.host.evaluate_string(source)?;
        self.checkpoint()?;
        Ok(value)
    }

    pub fn click(&mut self, selector: &str) -> Result<(), HostError> {
        let selector = serde_json::to_string(selector)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.execute(&format!(
            "const target=document.querySelector({selector});if(!target)throw new Error('no matching element');target.click()"
        ))
    }

    pub fn type_text(&mut self, selector: &str, value: &str) -> Result<(), HostError> {
        let node = {
            let dom = self.live_dom.borrow();
            dom.query_selector(dom.document(), selector)
        }
        .map_err(|error| HostError::Execution(error.to_string()))?
        .ok_or_else(|| HostError::Execution("no matching element".into()))?;
        self.live_dom
            .borrow_mut()
            .set_value(node, value, false)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.execute_with_node(
            node,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        )?;
        self.checkpoint()
    }

    pub fn location(&mut self) -> Result<String, HostError> {
        self.host.evaluate_string("location.href")
    }

    pub fn storage_snapshot(&mut self) -> Result<(String, String), HostError> {
        Ok((
            self.host
                .evaluate_string("JSON.stringify([...localStorage.items])")?,
            self.host
                .evaluate_string("JSON.stringify([...sessionStorage.items])")?,
        ))
    }

    pub fn take_console_entries(&mut self) -> Result<Vec<ConsoleEntry>, HostError> {
        let value = self
            .host
            .evaluate_string("JSON.stringify(__oryn_take_console())")?;
        serde_json::from_str(&value).map_err(|error| HostError::Execution(error.to_string()))
    }

    pub fn take_context_trace_entries(&mut self) -> Result<Vec<ContextTraceEntry>, HostError> {
        let value = self
            .host
            .evaluate_string("JSON.stringify(__oryn_take_context_trace())")?;
        serde_json::from_str(&value).map_err(|error| HostError::Execution(error.to_string()))
    }

    pub fn drain_tasks(&mut self) -> Result<(), HostError> {
        self.host.drain_scheduled_timers(true)?;
        self.host.execute("__oryn_poll_context()")?;
        self.checkpoint()
    }

    pub fn poll_context_events(&mut self) -> Result<(), HostError> {
        self.host.execute("__oryn_poll_context()")?;
        self.checkpoint()
    }

    pub fn apply_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        self.apply_action_with_sensitivity(target, action, value, false)
    }

    pub fn apply_sensitive_action(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        self.apply_action_with_sensitivity(target, action, value, true)
    }

    pub fn set_files(
        &mut self,
        target: NodeId,
        files: &[crate::upload::UploadFile],
    ) -> Result<(), HostError> {
        #[derive(Serialize)]
        struct PageFile<'a> {
            filename: &'a str,
            media_type: &'a str,
            body_base64: String,
            last_modified: u64,
        }
        let is_file_input = {
            let dom = self.live_dom.borrow();
            dom.node_name(target)
                .is_ok_and(|name| name.eq_ignore_ascii_case("input"))
                && dom
                    .attribute(target, "type")
                    .ok()
                    .flatten()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
        };
        if !is_file_input {
            return Err(HostError::Execution(
                "file selection target is not an input[type=file]".into(),
            ));
        }
        let page_files = files
            .iter()
            .map(|file| PageFile {
                filename: &file.filename,
                media_type: &file.media_type,
                body_base64: base64::engine::general_purpose::STANDARD.encode(&file.bytes),
                last_modified: 0,
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&page_files)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        {
            let mut dom = self.live_dom.borrow_mut();
            dom.set_attribute(target, "data-oryn-upload-files", encoded)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            dom.set_value(
                target,
                files.first().map_or("", |file| file.filename.as_str()),
                false,
            )
            .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        let realm = self
            .node_realms
            .get(&target)
            .copied()
            .unwrap_or(self.top_realm);
        self.host.activate_realm(realm)?;
        let result = self.host.execute_with_node(
            target,
            "__oryn_action_target",
            "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))",
        );
        self.host.activate_realm(self.top_realm)?;
        result?;
        self.checkpoint()
    }

    fn apply_action_with_sensitivity(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
        sensitive: bool,
    ) -> Result<(), HostError> {
        let submission_form = self.submission_form(target, action)?;
        let submits_form = submission_form.is_some();
        let event_source = match action {
            SemanticAction::Click => {
                let input_kind = {
                    let dom = self.live_dom.borrow();
                    dom.node_name(target)
                        .is_ok_and(|name| name.eq_ignore_ascii_case("input"))
                        .then(|| {
                            dom.attribute(target, "type")
                                .ok()
                                .flatten()
                                .unwrap_or("")
                                .to_ascii_lowercase()
                        })
                };
                if matches!(input_kind.as_deref(), Some("checkbox" | "radio")) {
                    let mut dom = self.live_dom.borrow_mut();
                    if input_kind.as_deref() == Some("checkbox")
                        && dom
                            .attribute(target, "checked")
                            .map_err(|error| HostError::Execution(error.to_string()))?
                            .is_some()
                    {
                        dom.remove_attribute(target, "checked")
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    } else {
                        dom.set_attribute(target, "checked", "")
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    }
                }
                ""
            }
            SemanticAction::Type | SemanticAction::Clear => {
                self.live_dom
                    .borrow_mut()
                    .set_value(
                        target,
                        if action == SemanticAction::Clear {
                            ""
                        } else {
                            value.unwrap_or_default()
                        },
                        sensitive,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                "__oryn_action_target.dispatchEvent(new Event('input',{bubbles:true}));__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Check | SemanticAction::Uncheck => {
                let mut dom = self.live_dom.borrow_mut();
                if action == SemanticAction::Check {
                    dom.set_attribute(target, "checked", "")
                } else {
                    dom.remove_attribute(target, "checked")
                }
                .map_err(|error| HostError::Execution(error.to_string()))?;
                "__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Select => {
                let requested = value.unwrap_or_default();
                let mut dom = self.live_dom.borrow_mut();
                let options = dom
                    .query_selector_all(target, "option")
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let selected = options.iter().copied().find(|option| {
                    dom.attribute(*option, "value")
                        .ok()
                        .flatten()
                        .is_some_and(|candidate| candidate == requested)
                        || dom
                            .arena()
                            .text_content(*option)
                            .is_ok_and(|text| text.trim() == requested)
                });
                let selected =
                    selected.ok_or_else(|| HostError::Execution("no matching option".into()))?;
                for option in options {
                    if option == selected {
                        dom.set_attribute(option, "selected", "")
                    } else {
                        dom.remove_attribute(option, "selected")
                    }
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                }
                "__oryn_action_target.dispatchEvent(new Event('change',{bubbles:true}))"
            }
            SemanticAction::Focus => "__oryn_action_target.focus()",
            SemanticAction::Hover => {
                "__oryn_action_target.dispatchEvent(new Event('mouseover',{bubbles:true}))"
            }
            SemanticAction::Submit => "",
            _ => {
                return Err(HostError::Execution(format!(
                    "unsupported native executable action {action:?}"
                )));
            }
        };
        let action_realm = self
            .node_realms
            .get(&target)
            .copied()
            .unwrap_or(self.top_realm);
        self.host.activate_realm(action_realm)?;
        let action_accepted = if action == SemanticAction::Click {
            self.host.evaluate_bool_with_node(
                target,
                "__oryn_action_target",
                "__oryn_action_target.dispatchEvent(new Event('click',{bubbles:true,cancelable:true}))",
            )
        } else {
            self.host
                .execute_with_node(target, "__oryn_action_target", event_source)
                .map(|()| true)
        };
        self.host.activate_realm(self.top_realm)?;
        let mut default_accepted = action_accepted?;
        if default_accepted && let Some(form) = submission_form {
            self.host.activate_realm(action_realm)?;
            let submit_accepted = self.host.evaluate_bool_with_node(
                form,
                "__oryn_submit_form",
                "__oryn_submit_form.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))",
            );
            self.host.activate_realm(self.top_realm)?;
            default_accepted = submit_accepted?;
        }
        if default_accepted && matches!(action, SemanticAction::Click | SemanticAction::Submit) {
            self.apply_default_navigation(target, action, submits_form)?;
        }
        self.checkpoint()
    }

    fn submission_form(
        &self,
        target: NodeId,
        action: SemanticAction,
    ) -> Result<Option<NodeId>, HostError> {
        let dom = self.live_dom.borrow();
        let target_record = dom
            .arena()
            .get(target)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let submits = match (&target_record.kind, action) {
            (NodeKind::Element { name }, SemanticAction::Submit) => name.local.as_ref() == "form",
            (NodeKind::Element { name }, SemanticAction::Click)
                if name.local.as_ref() == "button" =>
            {
                !target_record.attributes.get("type").is_some_and(|value| {
                    value.eq_ignore_ascii_case("button") || value.eq_ignore_ascii_case("reset")
                })
            }
            (NodeKind::Element { name }, SemanticAction::Click)
                if name.local.as_ref() == "input" =>
            {
                target_record.attributes.get("type").is_some_and(|value| {
                    value.eq_ignore_ascii_case("submit") || value.eq_ignore_ascii_case("image")
                })
            }
            _ => false,
        };
        if !submits {
            return Ok(None);
        }
        let mut candidate = Some(target);
        while let Some(node) = candidate {
            let record = dom
                .arena()
                .get(node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            if matches!(&record.kind, NodeKind::Element { name } if name.local.as_ref() == "form") {
                return Ok(Some(node));
            }
            candidate = record.parent;
        }
        Ok(None)
    }

    pub fn layout_snapshot(&mut self) -> Result<ParsedDocument, HostError> {
        self.checkpoint()?;
        Ok(self.document.clone())
    }

    pub fn install_layout(
        &mut self,
        output: &oryn_layout::LayoutOutput,
        viewport: oryn_common::v4::ViewportState,
    ) -> Result<(), HostError> {
        let geometry = output
            .geometry
            .iter()
            .map(|(node, value)| {
                use oryn_common::v4::GeometryAvailability;
                let value = match value.availability {
                    GeometryAvailability::Available => value.box_model.as_ref().map_or_else(
                        || {
                            oryn_js::v8_bindings::BindingGeometry::Unsupported(
                                "layout reported available geometry without a box".into(),
                            )
                        },
                        |model| {
                            let rect = model.border_box;
                            oryn_js::v8_bindings::BindingGeometry::Available(
                                oryn_js::v8_bindings::BindingRect {
                                    x: rect.x,
                                    y: rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                            )
                        },
                    ),
                    GeometryAvailability::Hidden => oryn_js::v8_bindings::BindingGeometry::Hidden,
                    GeometryAvailability::Unsupported => {
                        oryn_js::v8_bindings::BindingGeometry::Unsupported(
                            value
                                .diagnostic
                                .clone()
                                .unwrap_or_else(|| "geometry is unsupported".into()),
                        )
                    }
                };
                (*node, value)
            })
            .collect();
        self.host
            .install_layout(oryn_js::v8_bindings::BindingLayout {
                styles: output.computed_styles.clone(),
                geometry,
            })?;
        self.host.execute(&format!(
            "globalThis.innerWidth={};globalThis.innerHeight={}",
            viewport.width, viewport.height
        ))?;
        self.host.execute("__oryn_run_geometry_observers()")?;
        self.checkpoint()
    }

    pub fn run_animation_frame_phase(&mut self) -> Result<(), HostError> {
        self.host.execute("__oryn_run_animation_frames()")?;
        self.checkpoint()
    }

    pub fn install_media(
        &mut self,
        color_scheme: &str,
        reduced_motion: &str,
    ) -> Result<(), HostError> {
        let color_scheme = serde_json::to_string(color_scheme)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let reduced_motion = serde_json::to_string(reduced_motion)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.execute(&format!(
            "__oryn_media={{color_scheme:{color_scheme},reduced_motion:{reduced_motion}}}"
        ))
    }

    fn checkpoint(&mut self) -> Result<(), HostError> {
        self.host.drain_async_fetches(true)?;
        self.drain_dynamic_scripts()?;
        self.host.execute("void 0")?;
        self.host.execute("customElements.upgrade(document)")?;
        self.host.execute("__oryn_run_custom_reactions()")?;
        self.host.run_dom_observer_checkpoint()?;
        for rejection in self.host.take_unhandled_promise_rejections() {
            self.diagnostics
                .push(script_diagnostic(HostError::Execution(format!(
                    "unhandled promise rejection: {rejection}"
                ))));
        }
        self.host.process_binding_releases()?;
        let (dom, document) = self.live_dom.borrow().snapshot();
        self.document.dom = dom;
        self.document.document = document;
        Ok(())
    }

    fn drain_dynamic_scripts(&mut self) -> Result<(), HostError> {
        // Script execution may itself connect another script. Keep one bounded
        // preparation loop so recursive insertion is deterministic and cannot
        // monopolize the page worker indefinitely.
        for _ in 0..1024 {
            let pending = self.host.take_pending_dynamic_scripts()?;
            if pending.is_empty() {
                return Ok(());
            }
            for script in pending {
                let (script_type, src, nonce, inline) = {
                    let dom = self.live_dom.borrow();
                    let script_type = dom
                        .attribute(script, "type")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .unwrap_or_default()
                        .to_owned();
                    let src = dom
                        .attribute(script, "src")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned);
                    let nonce = dom
                        .attribute(script, "nonce")
                        .map_err(|error| HostError::Execution(error.to_string()))?
                        .map(str::to_owned);
                    let inline = dom
                        .arena()
                        .text_content(script)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    (script_type, src, nonce, inline)
                };
                let module = script_type.trim().eq_ignore_ascii_case("module");
                let loaded = self.load_dynamic_script_source(
                    src.as_deref(),
                    module,
                    nonce.as_deref(),
                    &inline,
                );
                let result = loaded.and_then(|(resource_name, source)| {
                    if module {
                        self.host
                            .execute_module(&resource_name, &source, &self.resources)
                    } else {
                        self.host.execute(&source)
                    }
                });
                match result {
                    Ok(()) => {
                        self.host.execute_with_node(
                            script,
                            "__oryn_dynamic_script",
                            "__oryn_dynamic_script.dispatchEvent(new Event('load'))",
                        )?;
                    }
                    Err(error) => {
                        self.diagnostics
                            .push(script_diagnostic(HostError::Execution(format!(
                                "dynamic script failed: {error}"
                            ))));
                        // Resource/script errors are reported to the element
                        // and diagnostics but do not turn appendChild into an
                        // exception after it has already returned.
                        let _ = self.host.execute_with_node(
                            script,
                            "__oryn_dynamic_script",
                            "__oryn_dynamic_script.dispatchEvent(new Event('error'))",
                        );
                    }
                }
            }
        }
        Err(HostError::LimitExceeded {
            resource: "dynamic script preparation queue".into(),
        })
    }

    fn load_dynamic_script_source(
        &mut self,
        src: Option<&str>,
        module: bool,
        nonce: Option<&str>,
        inline: &str,
    ) -> Result<(String, String), HostError> {
        let Some(src) = src.filter(|src| !src.trim().is_empty()) else {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &self.context {
                client
                    .request(
                        crate::context_service::ContextServiceRequest::AuthorizeInlineScript {
                            page_id: *page_id,
                            source: inline.to_owned(),
                            nonce: nonce.map(str::to_owned),
                        },
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
            return Ok(("oryn:dynamic-inline-module".into(), inline.to_owned()));
        };

        #[cfg(unix)]
        if let Some((client, page_id, _)) = &self.context {
            let response = client
                .request(crate::context_service::ContextServiceRequest::LoadScript {
                    page_id: *page_id,
                    url: src.to_owned(),
                    module,
                    nonce: nonce.map(str::to_owned),
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let crate::context_service::ContextServiceResponse::ScriptSource {
                url, source, ..
            } = response
            else {
                return Err(HostError::Execution(
                    "parent returned an invalid dynamic-script response".into(),
                ));
            };
            return Ok((url, source));
        }

        let resolved = self
            .base_url
            .as_ref()
            .and_then(|base| base.join(src).ok())
            .or_else(|| Url::parse(src).ok());
        if let Some(source) = self.resources.get(src).cloned().or_else(|| {
            resolved
                .as_ref()
                .and_then(|url| self.resources.get(url.as_str()).cloned())
        }) {
            return Ok((
                resolved.map_or_else(|| src.to_owned(), |url| url.to_string()),
                source,
            ));
        }
        let url = resolved.ok_or_else(|| {
            HostError::Execution(format!("dynamic script URL {src:?} is invalid"))
        })?;
        let broker = self.broker.as_ref().ok_or_else(|| {
            HostError::Execution(format!("dynamic script resource {url} is unavailable"))
        })?;
        let request_id = self.next_dynamic_request_id;
        self.next_dynamic_request_id = request_id.saturating_add(1);
        let response = broker
            .send(crate::network::NetworkRequest {
                request_id,
                method: "GET".into(),
                url: url.to_string(),
                headers: vec![(
                    "accept".into(),
                    "text/javascript,application/javascript,*/*;q=0.1".into(),
                )],
                body: Vec::new(),
                credentials: true,
                security: crate::cookies::CookieRequestContext {
                    kind: crate::cookies::CookieRequestKind::Script,
                    initiator_url: self.base_url.as_ref().map(ToString::to_string),
                    top_level_url: self.base_url.as_ref().map(ToString::to_string),
                    ..Default::default()
                },
                resolved_addrs: Vec::new(),
            })
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        let (source, _) = oryn_net::decode_resource_text(
            &response.body,
            content_type,
            oryn_net::TextResourceKind::JavaScript,
        );
        Ok((url.to_string(), source))
    }

    pub fn node_by_id(&self, id: &str) -> Option<NodeId> {
        self.live_dom.borrow().find_by_id(id)
    }

    fn apply_default_navigation(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        submits_form: bool,
    ) -> Result<(), HostError> {
        let (href, target_name) = {
            let dom = self.live_dom.borrow();
            let mut candidate = Some(target);
            let mut result = None;
            while let Some(node) = candidate {
                let record = dom
                    .arena()
                    .get(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if let NodeKind::Element { name } = &record.kind {
                    if name.local.as_ref() == "a"
                        && action == SemanticAction::Click
                        && let Some(href) = record.attributes.get("href")
                    {
                        result = Some((
                            href.clone(),
                            record.attributes.get("target").cloned().unwrap_or_default(),
                        ));
                        break;
                    }
                    if name.local.as_ref() == "form"
                        && (action == SemanticAction::Submit || submits_form)
                    {
                        result = Some((
                            record.attributes.get("action").cloned().unwrap_or_default(),
                            record.attributes.get("target").cloned().unwrap_or_default(),
                        ));
                        break;
                    }
                }
                candidate = record.parent;
            }
            let Some(result) = result else {
                return Ok(());
            };
            result
        };
        let resolved = self
            .base_url
            .as_ref()
            .and_then(|base| base.join(&href).ok())
            .or_else(|| Url::parse(&href).ok())
            .map(|url| url.to_string())
            .unwrap_or(href);
        if target_name.eq_ignore_ascii_case("_blank") {
            #[cfg(unix)]
            if let Some((client, page_id, _)) = &self.context {
                let response = client
                    .request(crate::context_service::ContextServiceRequest::OpenPage {
                        opener_page_id: *page_id,
                        url: Some(resolved),
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                match response {
                    crate::context_service::ContextServiceResponse::PageOpened {
                        page_id: opened_page_id,
                    } => {
                        self.host.execute(&format!(
                            "__oryn_context_trace.push({{kind:'page_created',detail:{{page_id:{},opener_page_id:{}}}}})",
                            opened_page_id.0, page_id.0
                        ))?;
                    }
                    crate::context_service::ContextServiceResponse::PageDenied { reason } => {
                        let reason = serde_json::to_string(&reason)
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        self.host.execute(&format!(
                            "__oryn_context_trace.push({{kind:'capability_failure',detail:{{capability:'popup.open',detail:{reason}}}}})"
                        ))?;
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        let resolved = serde_json::to_string(&resolved)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.host.execute(&format!("location.href={resolved}"))
    }
}

fn script_diagnostic(error: HostError) -> oryn_common::v3::CapabilityDiagnostic {
    oryn_common::v3::CapabilityDiagnostic {
        capability: "javascript.execution".into(),
        support: oryn_common::v3::SupportLevel::Partial,
        alternatives: vec![
            oryn_common::v3::ExecutionDomain::Chromium,
            oryn_common::v3::ExecutionDomain::Webkit,
            oryn_common::v3::ExecutionDomain::UserBrowser,
        ],
        handoff_lossy: true,
        detail: Some(error.to_string()),
    }
}

fn subtree_nodes(dom: &DomRuntime, root: NodeId) -> Result<Vec<NodeId>, HostError> {
    let mut nodes = vec![root];
    let mut index = 0;
    while index < nodes.len() {
        let children = dom
            .arena()
            .get(nodes[index])
            .map_err(|error| HostError::Execution(error.to_string()))?
            .children
            .clone();
        nodes.extend(children);
        index += 1;
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_and_events_mutate_only_the_rust_dom() {
        let mut page = RustExecutableDocument::load(
            "<main id='root'></main><script>const button=document.createElement('button');button.id='go';button.addEventListener('click',()=>button.textContent='done');document.getElementById('root').appendChild(button)</script>",
        )
        .unwrap();
        page.click("#go").unwrap();
        let button = page.node_by_id("go").unwrap();
        assert_eq!(page.document.dom.text_content(button).unwrap(), "done");
    }

    #[test]
    fn connected_inline_dynamic_script_executes_before_append_child_returns() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>globalThis.order=[];const dynamic=document.createElement('script');dynamic.textContent=\"order.push('dynamic')\";document.querySelector('body').appendChild(dynamic);order.push('after')</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("order.join(',')").unwrap(), "dynamic,after");
    }

    #[test]
    fn connected_external_dynamic_script_uses_the_resource_scheduler_once() {
        let resources = BTreeMap::from([(
            "dynamic.js".into(),
            "globalThis.dynamicRuns=(globalThis.dynamicRuns||0)+1".into(),
        )]);
        let mut page = RustExecutableDocument::load_with_resources(
            "<body><main></main><script>const dynamic=document.createElement('script');dynamic.setAttribute('src','dynamic.js');dynamic.addEventListener('load',()=>globalThis.dynamicLoaded=true);const body=document.querySelector('body');body.appendChild(dynamic);body.removeChild(dynamic);body.appendChild(dynamic)</script></body>",
            &resources,
        )
        .unwrap();
        assert_eq!(page.evaluate("String(dynamicRuns)").unwrap(), "1");
        assert_eq!(page.evaluate("String(dynamicLoaded)").unwrap(), "true");
    }

    #[test]
    fn inner_html_scripts_remain_inert_when_the_subtree_is_connected() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>globalThis.ran=false;const holder=document.createElement('div');holder.innerHTML='<script>ran=true<\\/script>';document.querySelector('body').appendChild(holder)</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("String(ran)").unwrap(), "false");
    }

    #[test]
    fn connected_dynamic_module_uses_v8_module_evaluation() {
        let mut page = RustExecutableDocument::load(
            "<body><main></main><script>const dynamic=document.createElement('script');dynamic.setAttribute('type','module');dynamic.textContent='globalThis.dynamicModule=41+1;export const value=42';document.querySelector('body').appendChild(dynamic)</script></body>",
        )
        .unwrap();
        assert_eq!(page.evaluate("String(dynamicModule)").unwrap(), "42");
    }

    #[test]
    fn event_current_target_is_the_generated_rust_dom_wrapper() {
        let mut page = RustExecutableDocument::load(
            "<button id=go>Run</button><script>const button=document.getElementById('go');button.addEventListener('click',event=>globalThis.__sameTarget=event.currentTarget===button)</script>",
        )
        .unwrap();
        page.click("#go").unwrap();
        assert_eq!(page.evaluate("String(__sameTarget)").unwrap(), "true");
    }

    #[test]
    fn parser_inserted_document_write_mutates_the_rust_dom_at_the_script_anchor() {
        let page = RustExecutableDocument::load(
            "<main id=before>before</main><script>document.write('<section id=written>inserted</section>')</script><footer id=after>after</footer>",
        )
        .unwrap();
        let written = page.node_by_id("written").expect("written node");
        assert_eq!(page.document.dom.text_content(written).unwrap(), "inserted");
        let body = page
            .document
            .dom
            .iter()
            .find(|(_, node)| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "body")
            })
            .map(|(node, _)| node)
            .unwrap();
        let ids = page
            .document
            .dom
            .get(body)
            .unwrap()
            .children
            .iter()
            .filter_map(|node| {
                page.document
                    .dom
                    .get(*node)
                    .ok()?
                    .attributes
                    .get("id")
                    .cloned()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["before", "written", "after"]);
    }

    #[test]
    fn parser_blocking_script_cannot_observe_later_source_markup() {
        let page = RustExecutableDocument::load(
            "<output id=result></output><script>document.getElementById('result').textContent=String(document.getElementById('later')===null)</script><aside id=later>later</aside>",
        )
        .unwrap();
        let result = page.node_by_id("result").expect("result node");
        assert_eq!(page.document.dom.text_content(result).unwrap(), "true");
        assert!(page.node_by_id("later").is_some());
    }

    #[test]
    fn parser_inserted_scripts_can_nest_document_write_with_bounded_reentry() {
        let page = RustExecutableDocument::load(
            r#"<script>document.write('<script>document.write(\"<output id=nested>nested</output>\")<\/script>')</script>"#,
        )
        .unwrap();
        let nested = page.node_by_id("nested").expect("nested written node");
        assert_eq!(page.document.dom.text_content(nested).unwrap(), "nested");
    }

    #[test]
    fn document_open_write_close_replaces_contents_without_a_javascript_dom_copy() {
        let page = RustExecutableDocument::load(
            "<p id=old>old</p><script>document.open();document.write('<title>new</title><main id=fresh>fresh</main>');document.close()</script>",
        )
        .unwrap();
        assert!(page.node_by_id("old").is_none());
        let fresh = page.node_by_id("fresh").expect("replacement node");
        assert_eq!(page.document.dom.text_content(fresh).unwrap(), "fresh");
    }

    #[test]
    fn lifecycle_events_run_after_parser_inserted_scripts() {
        let page = RustExecutableDocument::load(
            "<output id=state>initial</output><script>document.addEventListener('DOMContentLoaded',()=>document.querySelector('#state').textContent+='|dom');addEventListener('load',()=>document.querySelector('#state').textContent+='|load')</script>",
        )
        .unwrap();
        let state = page.node_by_id("state").unwrap();
        assert_eq!(
            page.document.dom.text_content(state).unwrap(),
            "initial|dom|load"
        );
    }

    #[test]
    fn timers_wait_for_deadlines_and_checkpoint_microtasks_between_tasks() {
        let mut page = RustExecutableDocument::load(
            "<output id=state>pending</output><script>globalThis.order=[];setTimeout(()=>{order.push('timer-1');Promise.resolve().then(()=>order.push('microtask-1'))},15);setTimeout(()=>order.push('timer-2'),15)</script>",
        )
        .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#state').textContent")
                .unwrap(),
            "pending"
        );
        let started = std::time::Instant::now();
        page.drain_tasks().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(10));
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "timer-1,microtask-1,timer-2"
        );
    }

    #[test]
    fn interval_can_cancel_itself_without_becoming_an_infinite_drain() {
        let mut page = RustExecutableDocument::load(
            "<script>globalThis.count=0;const id=setInterval(()=>{count++;clearInterval(id)},1)</script>",
        )
        .unwrap();
        page.drain_tasks().unwrap();
        page.drain_tasks().unwrap();
        assert_eq!(page.evaluate("String(count)").unwrap(), "1");
    }

    #[test]
    fn ordinary_button_click_does_not_schedule_document_navigation() {
        let mut page = RustExecutableDocument::load(
            "<button type=button id=go onclick=\"document.querySelector('#status').textContent='changed'\">Mutate</button><output id=status>before</output>",
        )
        .unwrap();
        let button = page.node_by_id("go").expect("button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("ordinary button click must not navigate");
        let status = page.node_by_id("status").expect("status");
        assert_eq!(page.document.dom.text_content(status).unwrap(), "changed");
        assert_eq!(
            page.evaluate("new URL('fixture:///tmp/page.html').href")
                .unwrap(),
            "fixture:///tmp/page.html"
        );
    }

    #[test]
    fn parsed_inline_location_assignment_updates_the_committed_url() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<button type=button id=go onclick=\"window.location.href='/next.html'\">Next</button>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/start.html").unwrap()),
        )
        .unwrap();
        let button = page.node_by_id("go").expect("button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("inline navigation handler");
        assert_eq!(page.location().unwrap(), "https://example.test/next.html");
    }

    #[test]
    fn shipping_fixture_back_button_updates_location() {
        let mut page = RustExecutableDocument::load_with_environment(
            include_str!("../../../test-harness/scenarios/intent-tests/flow-shipping.html"),
            &BTreeMap::new(),
            None,
            Some(Url::parse("http://localhost:3000/intent-tests/flow-shipping.html").unwrap()),
        )
        .unwrap();
        let button = {
            let dom = page.live_dom.borrow();
            dom.arena()
                .iter()
                .find(|(id, node)| {
                    dom.is_connected(*id).unwrap_or(false)
                        && node
                            .attributes
                            .get("onclick")
                            .is_some_and(|source| source.contains("flow-cart.html"))
                })
                .map(|(node, _)| node)
                .expect("back button")
        };
        page.apply_action(button, SemanticAction::Click, None)
            .expect("inline back navigation");
        assert_eq!(
            page.location().unwrap(),
            "http://localhost:3000/intent-tests/flow-cart.html"
        );
    }

    #[test]
    fn document_title_reflects_the_rust_owned_title_element() {
        let mut page = RustExecutableDocument::load("<title>Initial</title><main>Body</main>")
            .expect("document");
        assert_eq!(page.evaluate("document.title").unwrap(), "Initial");
        page.execute("document.title='Updated'").unwrap();
        assert_eq!(page.evaluate("document.title").unwrap(), "Updated");
        assert_eq!(
            page.evaluate("document.querySelector('title').textContent")
                .unwrap(),
            "Updated"
        );
    }

    #[test]
    fn prevented_submit_event_keeps_the_document_and_publishes_handler_mutations() {
        let mut page = RustExecutableDocument::load(
            r#"<form id=form action=/charged><button id=submit type=submit>Sign In</button></form>
            <div id=modal style="display:none"></div>
            <script>document.getElementById('form').onsubmit=e=>{e.preventDefault();document.getElementById('modal').style.display='flex'}</script>"#,
        )
        .expect("document");
        let button = page.node_by_id("submit").expect("submit button");
        page.apply_action(button, SemanticAction::Click, None)
            .expect("prevented submit must not navigate");
        assert_eq!(page.evaluate("location.href").unwrap(), "about:blank");
        assert_eq!(
            page.evaluate("document.getElementById('modal').style.display")
                .unwrap(),
            "flex"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_owned_storage_is_visible_to_page_scripts() {
        let page = crate::runtime::Browser::new().new_context().new_page();
        page.load_html(
            "https://storage.example.test/app",
            "<script>localStorage.setItem('shared','value');sessionStorage.setItem('tab','one')</script>",
        )
        .await
        .unwrap();
        let values = page
            .evaluate("localStorage.getItem('shared')+'|'+sessionStorage.getItem('tab')")
            .await
            .unwrap();
        assert_eq!(values, "value|one", "trace: {:#?}", page.trace().await);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn denied_popup_is_reported_as_a_capability_failure() {
        let context = crate::runtime::Browser::new()
            .new_context_with_options(crate::runtime::ContextOptions::default().with_popups(false));
        let page = context.new_page();
        page.load_html("https://popup.example.test/app", "<main></main>")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("String(open('/denied'))").await.unwrap(),
            "null"
        );
        let trace = page.trace().await.unwrap();
        assert!(
            trace.iter().any(|entry| matches!(
                &entry.kind,
                crate::trace::TraceEventKind::PolicyDenied { capability }
                    if capability == "popup.open"
            )),
            "{trace:#?}"
        );
    }

    #[test]
    fn response_stream_delivers_every_buffered_chunk() {
        let mut page = RustExecutableDocument::load(
            "<output id=result>pending</output><script>const response=new Response(new Uint8Array([1,2,3]),{bodyChunks:[new Uint8Array([1]),new Uint8Array([2]),new Uint8Array([3])]});(async()=>{const reader=response.body.getReader();let chunks=0,total=0;for(;;){const item=await reader.read();if(item.done)break;chunks++;total+=item.value.length}document.querySelector('#result').textContent=chunks+':'+total})()</script>",
        )
        .unwrap();
        page.execute("__oryn_drain(true)").unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .unwrap(),
            "3:3"
        );
    }

    #[test]
    fn mutation_observers_run_after_microtasks_and_reach_a_stable_checkpoint() {
        let mut page = RustExecutableDocument::load(
            r#"<div id=root></div><script>
            globalThis.order=[];
            const root=document.querySelector('#root');
            new MutationObserver(records=>{
              order.push('observer:'+records[0].type);
              if(!document.querySelector('#child')){
                const child=document.createElement('span');child.id='child';root.appendChild(child);
              }
            }).observe(root,{childList:true,subtree:true});
            Promise.resolve().then(()=>order.push('microtask'));
            root.appendChild(document.createElement('b'));
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "microtask,observer:childList,observer:childList"
        );
        assert_eq!(
            page.evaluate("String(!!document.querySelector('#child'))")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn custom_element_reactions_precede_mutation_observer_delivery() {
        let mut page = RustExecutableDocument::load(
            r#"<x-state></x-state><script>
            globalThis.order=[];
            const node=document.querySelector('x-state');
            new MutationObserver(()=>order.push('observer')).observe(node,{attributes:true});
            customElements.define('x-state',class extends HTMLElement{connectedCallback(){order.push('reaction');this.setAttribute('ready','')}});
            Promise.resolve().then(()=>order.push('microtask'));
            </script>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("order.join(',')").unwrap(),
            "microtask,reaction,observer"
        );
    }
}
