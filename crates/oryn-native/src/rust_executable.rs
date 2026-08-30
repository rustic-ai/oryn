//! Production executable page backed by the Rust-owned live DOM.
//!
//! This module never exports or reconstructs a JavaScript-owned node graph.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use oryn_common::v3::SemanticAction;
use oryn_dom::{DomRuntime, NodeId, NodeKind, html};
use oryn_js::realm::RealmId;
use serde::{Deserialize, Serialize};

use crate::{
    host::{HostError, JavaScriptHost, RawV8Host},
    html::ParsedDocument,
    network::SharedNetworkBroker,
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
globalThis.__oryn_context_trace=[];
globalThis.__oryn_take_context_trace=()=>__oryn_context_trace.splice(0);
globalThis.location={href:'about:blank'};
globalThis.__oryn_layout={styles:{},geometry:{},viewport:{},scroll:{},media:{}};
globalThis.__oryn_install_layout=value=>{globalThis.__oryn_layout=value};
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
globalThis.__oryn_timers=[];
globalThis.setTimeout=(callback,_delay=0,...args)=>{
  const id=__oryn_timers.length+1;
  __oryn_timers.push({id,callback,args,cancelled:false,interval:false});
  return id;
};
globalThis.clearTimeout=id=>{const timer=__oryn_timers.find(timer=>timer.id===id);if(timer)timer.cancelled=true};
globalThis.setInterval=(callback,_delay=0,...args)=>{
  const id=__oryn_timers.length+1;
  __oryn_timers.push({id,callback,args,cancelled:false,interval:true});
  return id;
};
globalThis.clearInterval=globalThis.clearTimeout;
globalThis.__oryn_drain_timers=()=>{
  const ready=__oryn_timers.splice(0);
  for(const timer of ready){
    if(timer.cancelled)continue;
    timer.callback(...timer.args);
    if(timer.interval&&!timer.cancelled)__oryn_timers.push(timer);
  }
};
globalThis.__oryn_drain=()=>{__oryn_drain_timers();if(typeof __oryn_poll_context==='function')__oryn_poll_context()};
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
  constructor(input,base){input=String(input);if(base&&!/^[a-z][a-z0-9+.-]*:/i.test(input)){const parent=new OrynURL(base);const root=parent.host?parent.origin:parent.protocol;if(input.startsWith('#'))input=root+parent.pathname+parent.search+input;else if(input.startsWith('?'))input=root+parent.pathname+input;else input=input.startsWith('/')?root+input:root+parent.pathname.replace(/[^/]*$/,'')+input}const match=input.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i);if(!match)throw new TypeError('Invalid URL');this.protocol=match[1];this.host=match[2]||'';this.hostname=this.host.split(':')[0];this.port=this.host.slice(this.hostname.length+1);this.pathname=match[3]||'/';this.search=match[4]||'';this.hash=match[5]||'';this.origin=this.host?this.protocol+'//'+this.host:'null';this.searchParams=new OrynURLSearchParams(this.search)}
  get href(){const query=this.searchParams.toString();return (this.host?this.origin:this.protocol)+this.pathname+(query?'?'+query:'')+this.hash}
  set href(value){Object.assign(this,new OrynURL(value,this.href))}
  toString(){return this.href}
  toJSON(){return this.href}
}
globalThis.URL=OrynURL;globalThis.URLSearchParams=OrynURLSearchParams;
globalThis.location=new OrynURL(location.href);
const contextCall=request=>JSON.parse(globalThis.__oryn_context_call(JSON.stringify(request)));
const __oryn_page_proxy=pageId=>pageId==null?null:({__orynPageId:pageId,__closed:false,get closed(){return this.__closed},focus(){contextCall({operation:'activate_page',source_page_id:__oryn_page_id,target_page_id:pageId})},close(){contextCall({operation:'close_page',source_page_id:__oryn_page_id,target_page_id:pageId});this.__closed=true},postMessage(data,targetOrigin='*'){contextCall({operation:'post_message',source_page_id:__oryn_page_id,target_page_id:pageId,target_origin:String(targetOrigin),data})}});
globalThis.__oryn_configure_context=(pageId,openerPageId)=>{__oryn_page_id=pageId;globalThis.opener=__oryn_page_proxy(openerPageId)};
globalThis.open=(url='')=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return null;const target=url?new OrynURL(url,location.href).href:null;const response=contextCall({operation:'open_page',opener_page_id:__oryn_page_id,url:target});if(response.result==='page_opened'){__oryn_context_trace.push({kind:'page_created',detail:{page_id:response.page_id,opener_page_id:__oryn_page_id}});return __oryn_page_proxy(response.page_id)}if(response.result==='page_denied')__oryn_context_trace.push({kind:'capability_failure',detail:{capability:'popup.open',detail:response.reason}});return null};
const __oryn_global_listeners=new Map();const __oryn_websockets=new Set();let __oryn_context_event_id=0;
globalThis.addEventListener=(type,callback)=>{const list=__oryn_global_listeners.get(String(type))||[];list.push(callback);__oryn_global_listeners.set(String(type),list)};
globalThis.removeEventListener=(type,callback)=>__oryn_global_listeners.set(String(type),(__oryn_global_listeners.get(String(type))||[]).filter(item=>item!==callback));
globalThis.dispatchEvent=event=>{for(const callback of __oryn_global_listeners.get(event.type)||[])callback.call(globalThis,event);const handler=globalThis['on'+event.type];if(typeof handler==='function')handler.call(globalThis,event);return !event.defaultPrevented};
globalThis.__oryn_poll_context=()=>{if(typeof globalThis.__oryn_context_call!=='function'||__oryn_page_id===null)return;const response=contextCall({operation:'poll_events',page_id:__oryn_page_id,after_event_id:__oryn_context_event_id,max_events:32});if(response.result==='events')for(const record of response.events||[]){__oryn_context_event_id=Math.max(__oryn_context_event_id,record.event_id);const item=record.event;if(!item)continue;if(item.kind==='storage'){const change=item.change||{};const event=new Event('storage',{});event.key=change.key??null;event.oldValue=change.old_value??null;event.newValue=change.new_value??null;event.storageArea=change.area==='local'?localStorage:sessionStorage;event.url=location.href;dispatchEvent(event)}else if(item.kind==='message'){const event=new Event('message',{});event.data=item.data;event.origin=item.source_origin||'';event.source=__oryn_page_proxy(item.source_page_id);dispatchEvent(event)}}for(const socket of [...__oryn_websockets])socket.__poll()};
class OrynWebSocket {constructor(url,protocols=[]){this.url=new OrynURL(url,location.href).href;this.protocol='';this.binaryType='blob';this.readyState=0;this.__listeners=new Map();if(typeof protocols==='string')protocols=[protocols];const response=contextCall({operation:'web_socket_open',page_id:__oryn_page_id,url:this.url,protocols:[...protocols].map(String)});if(response.result!=='web_socket_opened')throw new Error('WebSocket open failed');this.__id=response.socket_id;this.protocol=response.protocol;__oryn_websockets.add(this);queueMicrotask(()=>{this.readyState=1;this.__event('open')})}addEventListener(type,callback){const list=this.__listeners.get(String(type))||[];list.push(callback);this.__listeners.set(String(type),list)}__event(type,extra={}){const event=new Event(type,{});Object.assign(event,extra);for(const callback of this.__listeners.get(type)||[])callback.call(this,event);const handler=this['on'+type];if(typeof handler==='function')handler.call(this,event)}send(data){const frame=typeof data==='string'?{kind:'text',value:data}:{kind:'binary',base64:bytesToBase64(bodyBytes(data))};const bytes=typeof data==='string'?new TextEncoder().encode(data).length:bodyBytes(data).length;contextCall({operation:'web_socket_send',page_id:__oryn_page_id,socket_id:this.__id,frame});__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'outbound',frame_type:frame.kind,bytes,url:this.url}})}close(code=1000,reason=''){contextCall({operation:'web_socket_close',page_id:__oryn_page_id,socket_id:this.__id,code,reason:String(reason)});this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code,reason:String(reason),wasClean:true})}__poll(){if(this.readyState!==1)return;const response=contextCall({operation:'web_socket_poll',page_id:__oryn_page_id,socket_id:this.__id,max_frames:32});if(response.result!=='web_socket_events')return;for(const frame of response.frames||[]){let bytes=0;if(frame.kind==='text'){bytes=new TextEncoder().encode(frame.value).length;this.__event('message',{data:frame.value})}else if(frame.kind==='binary'){const value=base64ToBytes(frame.base64);bytes=value.length;this.__event('message',{data:this.binaryType==='arraybuffer'?value.buffer:new Blob([value])})}else if(frame.kind==='close'){this.readyState=3;__oryn_websockets.delete(this);this.__event('close',{code:frame.code??1005,reason:frame.reason||'',wasClean:true})}__oryn_context_trace.push({kind:'web_socket_frame',detail:{socket_id:this.__id,direction:'inbound',frame_type:frame.kind,bytes,url:this.url}})}}}
Object.assign(OrynWebSocket,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});Object.assign(OrynWebSocket.prototype,{CONNECTING:0,OPEN:1,CLOSING:2,CLOSED:3});globalThis.WebSocket=OrynWebSocket;
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
class OrynFormData {constructor(form){this.items=[];if(form)for(const node of [...form.querySelectorAll('input'),...form.querySelectorAll('textarea'),...form.querySelectorAll('select')]){const name=node.getAttribute('name');if(!name||node.disabled)continue;const type=(node.getAttribute('type')||'').toLowerCase();if((type==='checkbox'||type==='radio')&&!node.checked)continue;this.append(name,node.value)}}append(name,value,filename){if(filename!==undefined&&value instanceof OrynBlob)value=new OrynFile([value.__bytes],filename,{type:value.type});this.items.push([String(name),value instanceof OrynBlob?value:String(value)])}set(name,value,filename){this.delete(name);this.append(name,value,filename)}get(name){const item=this.items.find(item=>item[0]===String(name));return item?item[1]:null}getAll(name){return this.items.filter(item=>item[0]===String(name)).map(item=>item[1])}has(name){return this.items.some(item=>item[0]===String(name))}delete(name){this.items=this.items.filter(item=>item[0]!==String(name))}[Symbol.iterator](){return this.items[Symbol.iterator]()}}
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
  const method=String(init.method||'GET').toUpperCase();
  const headers=[...new OrynHeaders(init.headers||[])];
  const signal=init.signal||null;if(signal&&signal.aborted)return Promise.reject(signal.reason||new Error('AbortError'));
  const bytes=bodyBytes(init.body);
  __oryn_context_trace.push({kind:'request_started',detail:{method,url:String(input),bytes:bytes.length}});
  return globalThis.__oryn_fetch_async(String(input),method,bytesToBase64(bytes),JSON.stringify(headers),'base64',init.credentials||'same-origin').then(raw=>{
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
        let inline_handlers = parsed
            .dom
            .iter()
            .flat_map(|(id, node)| {
                node.attributes.iter().filter_map(move |(name, source)| {
                    name.strip_prefix("on")
                        .map(|event| (id, event.to_string(), source.clone()))
                })
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
            host.execute(&format!("location.href={url}"))?;
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
                host.execute(&format!("location.href={url}"))?;
            }
            #[cfg(unix)]
            if let Some((page_id, _)) = context_identity {
                host.execute(&format!("__oryn_configure_context({}, null)", page_id.0))?;
            }
            let child_handlers = subtree_nodes(&live_dom.borrow(), child_document)?
                .into_iter()
                .flat_map(|node| {
                    live_dom
                        .borrow()
                        .arena()
                        .get(node)
                        .ok()
                        .into_iter()
                        .flat_map(move |record| {
                            record.attributes.iter().filter_map(move |(name, source)| {
                                name.strip_prefix("on")
                                    .map(|event| (node, event.to_string(), source.clone()))
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            for (node, event, source) in child_handlers {
                let event = serde_json::to_string(&event)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                host.execute_with_node(
                    node,
                    "__oryn_inline_target",
                    &format!(
                        "__oryn_inline_target.addEventListener({event},function(event){{{source}}})"
                    ),
                )?;
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
        for (node, event, source) in inline_handlers {
            let event = serde_json::to_string(&event)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            host.execute_with_node(
                node,
                "__oryn_inline_target",
                &format!(
                    "__oryn_inline_target.addEventListener({event},function(event){{{source}}})"
                ),
            )?;
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
                !matches!(kind, "" | "text/javascript" | "application/javascript")
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
        host.execute("document.dispatchEvent(new Event('DOMContentLoaded',{bubbles:true}));dispatchEvent(new Event('load'))")?;
        host.drain_async_fetches(true)?;
        host.execute("void 0")?;
        for rejection in host.take_unhandled_promise_rejections() {
            diagnostics.push(script_diagnostic(HostError::Execution(format!(
                "unhandled promise rejection: {rejection}"
            ))));
        }
        host.process_binding_releases()?;
        let (dom, document) = live_dom.borrow().snapshot();
        Ok(Self {
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
            #[cfg(unix)]
            context: context_handle,
        })
    }

    pub fn execute(&mut self, source: &str) -> Result<(), HostError> {
        self.host.execute(source)?;
        self.checkpoint()
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, HostError> {
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
        self.host
            .execute("__oryn_drain_timers();__oryn_poll_context()")?;
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

    fn apply_action_with_sensitivity(
        &mut self,
        target: NodeId,
        action: SemanticAction,
        value: Option<&str>,
        sensitive: bool,
    ) -> Result<(), HostError> {
        let event_source = match action {
            SemanticAction::Click => "__oryn_action_target.click()",
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
            SemanticAction::Submit => "__oryn_action_target.requestSubmit()",
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
        let action_result =
            self.host
                .execute_with_node(target, "__oryn_action_target", event_source);
        self.host.activate_realm(self.top_realm)?;
        action_result?;
        if matches!(action, SemanticAction::Click | SemanticAction::Submit) {
            self.apply_default_navigation(target, action)?;
        }
        self.checkpoint()
    }

    pub fn layout_snapshot(&mut self) -> Result<ParsedDocument, HostError> {
        self.checkpoint()?;
        Ok(self.document.clone())
    }

    pub fn install_layout(&mut self, output: &oryn_layout::LayoutOutput) -> Result<(), HostError> {
        let rects = output
            .geometry
            .iter()
            .filter_map(|(node, geometry)| {
                geometry.box_model.as_ref().map(|model| {
                    let rect = model.border_box;
                    (
                        *node,
                        oryn_js::v8_bindings::BindingRect {
                            x: rect.x,
                            y: rect.y,
                            width: rect.width,
                            height: rect.height,
                        },
                    )
                })
            })
            .collect();
        self.host
            .install_layout(oryn_js::v8_bindings::BindingLayout {
                styles: output.computed_styles.clone(),
                rects,
            })
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
        self.host.execute("void 0")?;
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

    pub fn node_by_id(&self, id: &str) -> Option<NodeId> {
        self.live_dom.borrow().find_by_id(id)
    }

    fn apply_default_navigation(
        &mut self,
        target: NodeId,
        action: SemanticAction,
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
                        && matches!(action, SemanticAction::Click | SemanticAction::Submit)
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
}
