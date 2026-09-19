// Historical runner only; installed after explicit native authority.
const __oryn_testdriver_text_node=(node,last=false)=>{if(!node)return null;if(node.nodeType===3)return node;let fallback=null,children=[...(node.childNodes||[])];if(last)children.reverse();for(const child of children){const found=__oryn_testdriver_text_node(child,last);if(found&&!fallback)fallback=found;if(found&&String(found.data||'').trim())return found}return fallback};
const __oryn_testdriver_key_name=value=>({"\uE003":'Backspace',"\uE004":'Tab',"\uE007":'Enter',"\uE008":'Shift',"\uE009":'Control',"\uE00A":'Alt',"\uE010":'End',"\uE011":'Home',"\uE012":'ArrowLeft',"\uE013":'ArrowUp',"\uE014":'ArrowRight',"\uE015":'ArrowDown',"\uE017":'Delete',"\uE03D":'Meta'}[value]||value);
const __oryn_testdriver_keyboard=(target,value,pressed,repeat=false)=>{
  target=__oryn_testdriver_key_target(target);
  const key=__oryn_testdriver_key_name(String(value));
  const modifiers=globalThis.__oryn_testdriver_modifiers||(globalThis.__oryn_testdriver_modifiers=new Set());
  if(pressed&&['Control','Meta','Shift','Alt'].includes(key))modifiers.add(key);
  if(!pressed)modifiers.delete(key);
  const code=/^[a-z]$/i.test(key)?'Key'+key.toUpperCase():/^[0-9]$/.test(key)?'Digit'+key:key===' '?'Space':key;
  const keyCode=({Backspace:8,Tab:9,Enter:13,Shift:16,Control:17,Alt:18,Pause:19,CapsLock:20,Escape:27,' ':32,PageUp:33,PageDown:34,End:35,Home:36,ArrowLeft:37,ArrowUp:38,ArrowRight:39,ArrowDown:40,Insert:45,Delete:46,Meta:91,NumLock:144,ScrollLock:145,';':186,':':186,'=':187,'+':187,',':188,'<':188,'-':189,'_':189,'.':190,'>':190,'/':191,'?':191,'`':192,'~':192,'[':219,'{':219,'\\':220,'|':220,']':221,'}':221,"'":222,'"':222})[key]??(/^[a-z0-9]$/i.test(key)?key.toUpperCase().charCodeAt(0):/^F(?:[1-9]|1[0-2])$/.test(key)?111+Number(key.slice(1)):0);
  const event=new KeyboardEvent(pressed?'keydown':'keyup',{bubbles:true,cancelable:true,composed:true,key,code,repeat,keyCode,charCode:0,
    ctrlKey:modifiers.has('Control'),metaKey:modifiers.has('Meta'),altKey:modifiers.has('Alt'),shiftKey:modifiers.has('Shift')});
  __oryn_dispatch_user_input(target,pressed?'keydown':'keyup',false,event);
  if(!pressed){modifiers.delete(key);return}
  if(event.defaultPrevented)return;
  if(key==='Control'||key==='Meta'||key==='Shift'||key==='Alt'){modifiers.add(key);return}
  const command=modifiers.has('Control')||modifiers.has('Meta');
  if(command&&['c','x','v'].includes(key.toLowerCase()))throw new Error('Clipboard shortcuts require the typed native input action route');
  if(command&&key.toLowerCase()==='a'){
    document.execCommand('selectAll',false,'');return
  }
  const tag=String(target.tagName||'').toLowerCase();
  const textControl=tag==='textarea'||(tag==='input'&&!['button','checkbox','file','radio','reset','submit'].includes(String(target.type||'').toLowerCase()));
  if(textControl&&(key==='Backspace'||key==='Delete'||[...key].length===1)){
    const unavailable=()=>!target.isConnected||target.getClientRects().length===0||getComputedStyle(target).visibility==='hidden';
    if(unavailable())return;
    if([...key].length===1){
      const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);
      if(keypress.defaultPrevented||unavailable())return;
    }
    const inputType=key==='Backspace'?'deleteContentBackward':key==='Delete'?'deleteContentForward':'insertText';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType,data:[...key].length===1?key:null});target.dispatchEvent(before);
    if(before.defaultPrevented||unavailable())return;
    const current=String(target.value||'');
    let start=Number(target.selectionStart??target.__oryn_selection_start??current.length),end=Number(target.selectionEnd??target.__oryn_selection_end??start);
    start=Math.max(0,Math.min(current.length,start));end=Math.max(start,Math.min(current.length,end));
    let replacement='';
    if(key==='Backspace'&&start===end&&start>0)start--;else if(key==='Delete'&&start===end&&end<current.length)end++;else if([...key].length===1)replacement=key;
    const caret=__oryn_edit_control(target,replacement,start,end);
    target.__oryn_selection_start=target.__oryn_selection_end=caret;
    const input=new Event('input',{bubbles:true});input.inputType=inputType;input.data=replacement||null;target.dispatchEvent(input);return
  }
  if(key==='Enter'){
    let plaintext=false;
    const selection=document.getSelection(),startNode=selection?.rangeCount?selection.getRangeAt(0).startContainer:target;
    for(let element=startNode?.nodeType===1?startNode:startNode?.parentElement; element; element=element.parentElement){
      const editable=element.getAttribute?.('contenteditable')?.toLowerCase();
      if(editable==='false')break;
      if(editable==='plaintext-only')plaintext=true;
      else if(editable===''||editable==='true')plaintext=false;
    }
    if(document.designMode.toLowerCase()==='on')plaintext=false;
    const lineBreak=modifiers.has('Shift')||plaintext,inputType=lineBreak?'insertLineBreak':'insertParagraph';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand(lineBreak?'insertLineBreak':'insertParagraph')
  }
  else if(key==='Backspace'||key==='Delete'){
    const inputType=key==='Backspace'?'deleteContentBackward':'deleteContentForward';
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand(key==='Backspace'?'delete':'forwardDelete')
  }
  else if(key==='Home'||key==='End'){
    const text=__oryn_testdriver_text_node(target,key==='End');if(text){const selection=document.getSelection();selection.collapse(text,key==='Home'?0:text.data.length)}
  }else if([...key].length===1){
    const keypress=new Event('keypress',{bubbles:true,cancelable:true});keypress.key=key;keypress.code=event.code;target.dispatchEvent(keypress);if(keypress.defaultPrevented)return;
    const before=new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertText',data:key});target.dispatchEvent(before);
    if(!before.defaultPrevented)document.execCommand('insertText',false,key)
  }
};
const __oryn_testdriver_key_target=target=>target?__oryn_resolve_keyboard_target(target):target;
globalThis.__oryn_testdriver_send_keys=async(target,keys)=>{await Promise.resolve();if(!target)throw new TypeError('testdriver target is required');target=__oryn_testdriver_key_target(target);target.focus?.();keys=String(keys);if(String(target.tagName).toLowerCase()==='input'&&String(target.type).toLowerCase()==='file'){const response=contextCall({operation:'materialize_upload',page_id:__oryn_page_id,label:keys});if(response.result!=='upload_materialized')throw new Error('parent did not authorize the selected upload');__oryn_select_file_bytes(target,JSON.stringify([{filename:response.filename,media_type:response.media_type,body_base64:response.body_base64,last_modified:response.last_modified}]));target.dispatchEvent(new Event('input',{bubbles:true}));target.dispatchEvent(new Event('change',{bubbles:true}));return}let currentTarget=target;for(const key of [...keys]){__oryn_testdriver_keyboard(currentTarget,key,true);currentTarget=__oryn_testdriver_key_target(document.activeElement||target);__oryn_testdriver_keyboard(currentTarget,key,false)}};
const __oryn_pointer_down=target=>{if(!target)return;if(__oryn_dispatch_user_input(target,'pointerdown',false)&&__oryn_dispatch_user_input(target,'mousedown',false))__oryn_focus_from_pointer(target)};
const __oryn_pointer_up=target=>{if(!target)return;__oryn_dispatch_user_input(target,'pointerup',false);__oryn_dispatch_user_input(target,'mouseup',false)};

globalThis.__oryn_testdriver_keyboard=__oryn_testdriver_keyboard;
