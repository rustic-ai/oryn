/// Shared Observable runtime installed in both Window and Worker realms.
///
/// This implements the currently mapped WICG Observable surface without
/// depending on page-owned globals.  It is kept as one immutable bootstrap so
/// the two realm kinds cannot silently diverge while the generated binding
/// callback implementation is completed.
pub(crate) const OBSERVABLE_GLOBALS: &str = r#"
const __oryn_observable_token = {};
const __oryn_observable_report = error => {
  if(globalThis.__oryn_document_detached)return;
  try {
    const event = new Event('error', {cancelable:true});
    Object.defineProperties(event, {
      error:{value:error, enumerable:true},
      message:{value:String(error?.message ?? error), enumerable:true},
      filename:{value:'observable', enumerable:true},
      lineno:{value:1, enumerable:true},
      colno:{value:1, enumerable:true}
    });
    globalThis.dispatchEvent?.(event);
  } catch (_) {}
};
class OrynSubscriber {
  constructor(token, observer, signal) {
    if (token !== __oryn_observable_token) throw new TypeError('Illegal constructor');
    this.__observer = observer || {};
    this.__token = token;
    this.__controller = new AbortController();
    this.__teardowns = [];
    this.__active = true;
    if (signal) {
      if (signal.aborted) this.unsubscribe(signal.reason);
      else {
        const abort=()=>this.unsubscribe(signal.reason);
        if(Array.isArray(signal.__listeners))signal.__listeners.unshift(abort);
        else {
          if(!Object.prototype.hasOwnProperty.call(signal,'__oryn_observable_aborts'))Object.defineProperty(signal,'__oryn_observable_aborts',{value:[],configurable:true});
          signal.__oryn_observable_aborts.push(abort);
        }
      }
    }
  }
  __brand() { if(this?.__token!==__oryn_observable_token)throw new TypeError('Illegal invocation'); }
  get active() { this.__brand(); return this.__active; }
  get signal() { this.__brand(); return this.__controller.signal; }
  addTeardown(teardown) {
    this.__brand();if(arguments.length<1)throw new TypeError('one argument required');
    if (teardown == null) return;
    const callback = typeof teardown === 'function' ? teardown :
      typeof teardown.unsubscribe === 'function' ? () => teardown.unsubscribe() : null;
    if (!callback) throw new TypeError('teardown must be callable or subscribable');
    if (!this.active) { try { callback(); } catch (error) { __oryn_observable_report(error); } }
    else this.__teardowns.push(callback);
  }
  next(value) {
    this.__brand();if(arguments.length<1)throw new TypeError('one argument required');
    if (!this.active||globalThis.__oryn_document_detached) return;
    const callback = this.__observer.next;
    if (typeof callback === 'function') {
      try { callback.call(this.__observer, value); }
      catch (error) { this.unsubscribe(error); __oryn_observable_report(error); }
    }
  }
  error(error) {
    this.__brand();if(arguments.length<1)throw new TypeError('one argument required');
    if (!this.active) { __oryn_observable_report(error); return; }
    const callback = this.__observer.error;
    this.__close(error);
    if (typeof callback === 'function') {
      try { callback.call(this.__observer, error); } catch (next) { __oryn_observable_report(next); }
    } else __oryn_observable_report(error);
  }
  complete() {
    this.__brand();
    if (!this.active) return;
    const callback = this.__observer.complete;
    this.__close();
    if (typeof callback === 'function') {
      try { callback.call(this.__observer); } catch (error) { __oryn_observable_report(error); }
    }
  }
  unsubscribe(reason) { this.__brand();if (this.active) this.__close(reason); }
  __close(reason) {
    this.__active = false;
    this.__controller.abort(reason);
    const teardowns = this.__teardowns.splice(0).reverse();
    for (const teardown of teardowns) {
      try { teardown(); } catch (error) { __oryn_observable_report(error); }
    }
  }
}

class Observable {
  constructor(callback) {
    if (typeof callback !== 'function') throw new TypeError('Observable initializer must be callable');
    this.__callback = callback;
    this.__producer = null;
    this.__consumers = new Set();
  }
  subscribe(observer = {}, options = {}) {
    if (typeof observer === 'function') observer = {next: observer};
    if (observer == null || (typeof observer !== 'object' && typeof observer !== 'function'))
      throw new TypeError('observer must be an object');
    return this.__subscribe(observer, options);
  }
  __subscribe(observer, options = {}, created = null) {
    const consumer = new OrynSubscriber(__oryn_observable_token, observer, options?.signal);
    if (created) created(consumer);
    if(globalThis.__oryn_document_detached)return consumer;
    if (consumer.active) this.__consumers.add(consumer);
    consumer.addTeardown(()=>{
      this.__consumers.delete(consumer);
      if(!this.__consumers.size&&this.__producer?.active)this.__producer.unsubscribe(consumer.signal.reason);
    });
    if(this.__producer?.active)return consumer;
    const producer = new OrynSubscriber(__oryn_observable_token, {
      next:value=>{if(!globalThis.__oryn_document_detached)for(const item of [...this.__consumers])if(item.active)item.next(value)},
      error:error=>{if(!globalThis.__oryn_document_detached)for(const item of [...this.__consumers])item.error(error);this.__consumers.clear();this.__producer=null},
      complete:()=>{if(!globalThis.__oryn_document_detached)for(const item of [...this.__consumers])item.complete();this.__consumers.clear();this.__producer=null}
    });
    this.__producer=producer;
    if(!consumer.active)producer.unsubscribe(consumer.signal.reason);
    try { producer.addTeardown(this.__callback(producer)); }
    catch (error) { producer.error(error); }
    if(!producer.active)this.__producer=null;
    return consumer;
  }
  map(mapper) { const source=this; return new Observable(out => {
    let index=0; return source.subscribe({next(value){try{out.next(mapper(value,index++))}catch(error){out.error(error)}},error:error=>out.error(error),complete:()=>out.complete()},{signal:out.signal});
  })}
  filter(predicate) { const source=this; return new Observable(out => {
    let index=0; return source.subscribe({next(value){try{if(predicate(value,index++))out.next(value)}catch(error){out.error(error)}},error:error=>out.error(error),complete:()=>out.complete()},{signal:out.signal});
  })}
  take(count) { count=Number(count);count=Number.isNaN(count)||count===0?0:count<0?Number.MAX_SAFE_INTEGER:Math.trunc(count); const source=this; return new Observable(out => {
    if(count===0){out.complete();return}let seen=0;return source.subscribe({next(value){if(++seen<=count)out.next(value);if(seen===count)out.complete()},error:error=>out.error(error),complete:()=>out.complete()},{signal:out.signal});
  })}
  drop(count) { count=Number(count);count=Number.isNaN(count)||count===0?0:count<0?Number.MAX_SAFE_INTEGER:Math.trunc(count); const source=this; return new Observable(out => {
    let seen=0;return source.subscribe({next:value=>{if(seen++>=count)out.next(value)},error:error=>out.error(error),complete:()=>out.complete()},{signal:out.signal});
  })}
  takeUntil(notifier) { const source=this; return new Observable(out => {
    const stop=Observable.from(notifier).subscribe({next:()=>out.complete(),error:()=>out.complete()},{signal:out.signal});
    if(!out.active)return stop;
    const inner=source.subscribe({next:value=>out.next(value),error:error=>out.error(error),complete:()=>out.complete()},{signal:out.signal});
    return()=>{stop.unsubscribe();inner.unsubscribe()};
  })}
  flatMap(mapper) { const source=this; return new Observable(out => {
    let index=0,outerDone=false,inner=null;const queue=[];
    const finish=()=>{if(outerDone&&!inner&&queue.length===0)out.complete()};
    const advance=()=>{if(inner||!queue.length){finish();return}let mapped;try{mapped=mapper(queue.shift(),index++)}catch(error){out.error(error);return}const token={subscription:null,done:false};inner=token;const subscription=Observable.from(mapped).subscribe({next:item=>out.next(item),error:error=>out.error(error),complete(){token.done=true;if(inner===token)inner=null;advance()}},{signal:out.signal});token.subscription=subscription;if(token.done&&inner===token){inner=null;advance()}};
    const outer=source.subscribe({next(value){queue.push(value);advance()},error:error=>out.error(error),complete(){outerDone=true;finish()}},{signal:out.signal});
    return()=>{outer.unsubscribe();inner?.subscription?.unsubscribe();queue.length=0};
  })}
  switchMap(mapper) { const source=this; return new Observable(out => {
    let index=0,inner=null,outerDone=false;const finish=()=>{if(outerDone&&!inner)out.complete()};
    const outer=source.subscribe({next(value){inner?.subscription?.unsubscribe();let mapped;try{mapped=mapper(value,index++)}catch(error){out.error(error);return}const token={subscription:null,done:false};inner=token;let subscription;try{subscription=Observable.from(mapped).subscribe({next:item=>out.next(item),error:error=>out.error(error),complete(){token.done=true;if(inner===token)inner=null;finish()}},{signal:out.signal})}catch(error){out.error(error);return}token.subscription=subscription;if(token.done&&inner===token)inner=null},error:error=>out.error(error),complete(){outerDone=true;finish()}},{signal:out.signal});
    return()=>{outer.unsubscribe();inner?.subscription?.unsubscribe()};
  })}
  catch(callback) { const source=this; return new Observable(out => {
    let recovery=null;const initial=source.subscribe({next:value=>out.next(value),error(error){let value;try{value=Observable.from(callback(error))}catch(next){out.error(next);return}recovery=value.subscribe({next:item=>out.next(item),error:next=>out.error(next),complete:()=>out.complete()},{signal:out.signal})},complete:()=>out.complete()},{signal:out.signal});return()=>{initial.unsubscribe();recovery?.unsubscribe()};
  })}
  finally(callback) { const source=this; return new Observable(out => {
    let called=false;const finish=()=>{if(!called){called=true;try{callback()}catch(error){__oryn_observable_report(error)}}};const sub=source.subscribe({next:value=>out.next(value),error(error){finish();out.error(error)},complete(){finish();out.complete()}},{signal:out.signal});return()=>{sub.unsubscribe();finish()};
  })}
  inspect(observer={}) { if(typeof observer==='function')observer={next:observer};const source=this;return new Observable(out=>{
    let terminal=false;
    try{observer.subscribe?.()}catch(error){out.error(error);return}
    const abort=()=>{if(!terminal)try{observer.abort?.(out.signal.reason)}catch(error){__oryn_observable_report(error)}};
    if(Array.isArray(out.signal.__listeners))out.signal.__listeners.unshift(abort);else{if(!Object.prototype.hasOwnProperty.call(out.signal,'__oryn_observable_aborts'))Object.defineProperty(out.signal,'__oryn_observable_aborts',{value:[],configurable:true});out.signal.__oryn_observable_aborts.unshift(abort)}
    return source.subscribe({next(value){try{observer.next?.(value);out.next(value)}catch(error){terminal=true;out.error(error)}},error(error){terminal=true;try{observer.error?.(error)}catch(next){out.error(next);return}out.error(error)},complete(){terminal=true;try{observer.complete?.()}catch(error){out.error(error);return}out.complete()}},{signal:out.signal})
  }) }
  forEach(callback, options={}) { return this.__terminal(options,(out)=>({next(value,index){callback(value,index)},complete(){out.resolve(undefined)}})); }
  every(predicate, options={}) { return this.__terminal(options,(out)=>({next(value,index){if(!predicate(value,index)){out.resolve(false);out.stop()}},complete(){out.resolve(true)}})); }
  find(predicate, options={}) { return this.__terminal(options,(out)=>({next(value,index){if(predicate(value,index)){out.resolve(value);out.stop()}},complete(){out.resolve(undefined)}})); }
  first(options={}) { return this.__terminal(options,(out)=>({next(value){out.resolve(value);out.stop()},complete(){out.reject(new RangeError('Observable is empty'))}})); }
  last(options={}) { let seen=false,last;return this.__terminal(options,(out)=>({next(value){seen=true;last=value},complete(){seen?out.resolve(last):out.reject(new RangeError('Observable is empty'))}})); }
  some(predicate, options={}) { return this.__terminal(options,(out)=>({next(value,index){if(predicate(value,index)){out.resolve(true);out.stop()}},complete(){out.resolve(false)}})); }
  reduce(reducer, ...rest) { const hasInitial=rest.length>0,initial=rest[0],options=rest[1]||{};let seen=hasInitial,acc=initial;return this.__terminal(options,(out)=>({next(value,index){if(!seen){seen=true;acc=value}else acc=reducer(acc,value,index)},complete(){seen?out.resolve(acc):out.reject(new TypeError('Reduce of empty Observable'))}})); }
  toArray(options={}) { const values=[];return this.__terminal(options,(out)=>({next:value=>values.push(value),complete:()=>out.resolve(values)})); }
  __terminal(options, factory) { const source=this;return new Promise((resolve,reject)=>{let sub,index=0,settled=false;const out={resolve(value){if(!settled){settled=true;resolve(value)}},reject(error){if(!settled){settled=true;reject(error)}},stop(reason){sub?.unsubscribe(reason)}};let observer;try{observer=factory(out)}catch(error){reject(error);return}const signal=options?.signal;if(signal?.aborted){reject(signal.reason);return}if(signal)signal.addEventListener('abort',()=>out.reject(signal.reason),{once:true});source.__subscribe({next(value){if(!settled){try{observer.next?.(value,index++)}catch(error){out.reject(error);out.stop(error)}}},error:error=>out.reject(error),complete:()=>{if(!settled){try{observer.complete?.()}catch(error){out.reject(error)}}}},{signal},value=>sub=value);if(settled)sub.unsubscribe()}); }
  static from(value) {
    if(value instanceof Observable)return value;
    if(value===null||value===undefined||(typeof value!=='object'&&typeof value!=='function'))throw new TypeError('value is not observable');
    const asyncMethod=value[Symbol.asyncIterator];
    if(asyncMethod!=null){
      if(typeof asyncMethod!=='function')throw new TypeError('async iterator is not callable');
      const subscribers=new Set();let running=false;
      return new Observable(out=>{
        if(!out.active)return;
        subscribers.add(out);
        if(!running){
          running=true;
          let iterator,stopped=false;
          try{
            const method=value[Symbol.asyncIterator];
            if(method==null){const fallback=value[Symbol.iterator];if(typeof fallback!=='function')throw new TypeError('async iterator is not callable');iterator=fallback.call(value)}
            else{if(typeof method!=='function')throw new TypeError('async iterator is not callable');iterator=method.call(value)}
          }
          catch(error){running=false;for(const item of [...subscribers])item.error(error);subscribers.clear();return}
          if(!out.active){running=false;return}
          const close=()=>{if(stopped)return;stopped=true;running=false;if(typeof iterator.return==='function'){let returned;try{returned=iterator.return(out.signal.reason)}catch(error){Promise.reject(error);return}Promise.resolve(returned).then(value=>{if(typeof value!=='object'||value===null)throw new TypeError('Iterator return() must return an Object')})}};
          if(Array.isArray(out.signal.__listeners))out.signal.__listeners.unshift(close);else{if(!Object.prototype.hasOwnProperty.call(out.signal,'__oryn_observable_aborts'))Object.defineProperty(out.signal,'__oryn_observable_aborts',{value:[],configurable:true});out.signal.__oryn_observable_aborts.unshift(close)}
          (async()=>{try{await Promise.resolve();const next=iterator.next;if(typeof next!=='function')throw new TypeError('iterator next is not callable');while(!stopped){const item=await Promise.resolve(next.call(iterator));if(typeof item!=='object'||item===null)throw new TypeError('iterator result is not an object');if(item.done){for(const subscriber of [...subscribers])subscriber.complete();subscribers.clear();running=false;break}for(const subscriber of [...subscribers])if(subscriber.active)subscriber.next(item.value);for(const subscriber of [...subscribers])if(!subscriber.active)subscribers.delete(subscriber);if(!subscribers.size)close()}}catch(error){if(!stopped)for(const subscriber of [...subscribers])subscriber.error(error);subscribers.clear();running=false}})();
        }
        return()=>subscribers.delete(out);
      });
    }
    const iteratorMethod=value[Symbol.iterator];
    if(iteratorMethod!=null){
      if(typeof iteratorMethod!=='function')throw new TypeError('iterator is not callable');
      const subscribers=new Set();let running=false;
      return new Observable(out=>{
        if(!out.active)return;
        subscribers.add(out);
        if(!running){
          running=true;
          try{const method=value[Symbol.iterator];if(typeof method!=='function')throw new TypeError('iterator is not callable');const iterator=method.call(value);if(!out.active){running=false;return}const next=iterator.next;if(typeof next!=='function')throw new TypeError('iterator next is not callable');let stopped=false;const close=()=>{if(stopped)return;stopped=true;running=false;if(typeof iterator.return==='function'){const returned=iterator.return(out.signal.reason);if(typeof returned!=='object'||returned===null)throw new TypeError('Iterator return() must return an Object')}};if(Array.isArray(out.signal.__listeners))out.signal.__listeners.unshift(close);else{if(!Object.prototype.hasOwnProperty.call(out.signal,'__oryn_observable_aborts'))Object.defineProperty(out.signal,'__oryn_observable_aborts',{value:[],configurable:true});out.signal.__oryn_observable_aborts.unshift(close)}for(;;){const item=next.call(iterator);if(typeof item!=='object'||item===null)throw new TypeError('iterator result is not an object');if(item.done){stopped=true;for(const subscriber of [...subscribers])subscriber.complete();subscribers.clear();running=false;break}for(const subscriber of [...subscribers])if(subscriber.active)subscriber.next(item.value);for(const subscriber of [...subscribers])if(!subscriber.active)subscribers.delete(subscriber);if(!subscribers.size){close();break}}}catch(error){for(const subscriber of [...subscribers])subscriber.error(error);subscribers.clear();running=false}
        }
        return()=>subscribers.delete(out);
      });
    }
    if(value?.then)return new Observable(out=>{let cancelled=false;Promise.resolve(value).then(item=>{if(!cancelled){out.next(item);out.complete()}},error=>{if(!cancelled)out.error(error)});return()=>{cancelled=true}});
    throw new TypeError('value is not observable');
  }
}
Object.defineProperty(Observable.prototype, Symbol.toStringTag, {value:'Observable'});
Object.defineProperty(OrynSubscriber.prototype, Symbol.toStringTag, {value:'Subscriber'});
Object.defineProperties(OrynSubscriber,{name:{value:'Subscriber'},length:{value:0}});
for(const name of ['next','error','complete','addTeardown'])Object.defineProperty(OrynSubscriber.prototype,name,{...Object.getOwnPropertyDescriptor(OrynSubscriber.prototype,name),enumerable:true});
for(const name of ['active','signal'])Object.defineProperty(OrynSubscriber.prototype,name,{...Object.getOwnPropertyDescriptor(OrynSubscriber.prototype,name),enumerable:true});
Object.defineProperty(Observable.prototype,'subscribe',{...Object.getOwnPropertyDescriptor(Observable.prototype,'subscribe'),enumerable:true});
const __oryn_event_observable = function(type, options={}) {
  const target=this, realmFrame=globalThis.frameElement;
  return new Observable(subscriber=>{
    const listener=event=>{if(!realmFrame||!realmFrame.__oryn_child_detached)subscriber.next(event)};
    target.addEventListener(String(type),listener,options);
    return()=>target.removeEventListener(String(type),listener,options);
  });
};
if(globalThis.EventTarget?.prototype&&!globalThis.EventTarget.prototype.when)
  Object.defineProperty(globalThis.EventTarget.prototype,'when',{value:__oryn_event_observable,configurable:true,writable:true});
if(!globalThis.when)Object.defineProperty(globalThis,'when',{value:__oryn_event_observable,configurable:true,writable:true});
Object.defineProperty(globalThis,'Observable',{value:Observable,configurable:true,writable:true});
Object.defineProperty(globalThis,'Subscriber',{value:OrynSubscriber,configurable:true,writable:true});
"#;
