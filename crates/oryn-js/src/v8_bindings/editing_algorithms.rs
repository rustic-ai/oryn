//! Native editing algorithms suspended at owned mutation operations.
use super::*;
use editing_continuation::Context;

pub(super) async fn apply_block_command(
    context: &Context,
    command: &str,
    value: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(anchor) = state.selection_anchor.get() else {
        return Ok(());
    };
    let alignment = match command {
        "justifyleft" => "left",
        "justifycenter" => "center",
        "justifyright" => "right",
        "justifyfull" => "justify",
        _ => return apply_block_command_at_boundary(context, command, value, anchor).await,
    };
    clear_contained_alignment(context).await?;
    wrap_alignment_contents(context, alignment).await
}

pub(super) async fn apply_block_command_at_boundary(
    context: &Context,
    command: &str,
    value: &str,
    anchor: BoundaryPoint,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let block = nearest_editing_block(&state.dom.borrow(), anchor.container, host)?;
    match command {
        "indent" => {
            context.style(block, "margin-left", "40px").await?;
        }
        "outdent" => {
            context.style(block, "margin-left", "").await?;
        }
        "insertorderedlist" | "insertunorderedlist" => {
            let list = if command == "insertorderedlist" {
                "ol"
            } else {
                "ul"
            };
            let (list_node, custody) =
                insert_editing_element_with_list_custody(context, list, &[]).await?;
            let children = light_children(&state.dom.borrow(), list_node)?;
            if !children
                .iter()
                .all(|child| element_local_name(&state.dom.borrow(), *child) == Some("li"))
            {
                let Some(custody) = custody else {
                    return Ok(());
                };
                let revision = state.selection_revision.get();
                let backward = state.selection_backward.get();
                let selected = custody.published;
                let first = custody.first;
                let last = custody.last;
                context.lease(first)?;
                context.lease(last)?;
                let empty_start = boundary_length(&state.dom.borrow(), first)? == 0;
                let empty_end = boundary_length(&state.dom.borrow(), last)? == 0;
                let start = custody.start;
                let end = custody.end;
                let item = state.dom.borrow_mut().create_element("li");
                for child in children {
                    context
                        .append_with_endpoint_markers(item, child, &[&start, &end])
                        .await?;
                }
                context
                    .append_with_endpoint_markers(list_node, item, &[&start, &end])
                    .await?;
                if selected && state.selection_revision.get() == revision {
                    let start = start.range()?;
                    let end = end.range()?;
                    context.lease(first)?;
                    context.lease(last)?;
                    let a = if empty_start {
                        let (container, offset) =
                            relative_boundary(&state.dom.borrow(), first, false)?;
                        BoundaryPoint { container, offset }
                    } else {
                        BoundaryPoint {
                            container: start.start_container,
                            offset: start.start_offset,
                        }
                    };
                    let b = if empty_end {
                        let (container, offset) =
                            relative_boundary(&state.dom.borrow(), last, true)?;
                        BoundaryPoint { container, offset }
                    } else {
                        BoundaryPoint {
                            container: end.start_container,
                            offset: end.start_offset,
                        }
                    };
                    context.lease(a.container)?;
                    context.lease(b.container)?;
                    let dom = state.dom.borrow();
                    if dom.is_connected(a.container)?
                        && dom.is_connected(b.container)?
                        && editing_host_for_node(
                            &dom,
                            Some(a.container),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )? == Some(host)
                        && editing_host_for_node(
                            &dom,
                            Some(b.container),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )? == Some(host)
                        && compare_boundary_points(
                            &dom,
                            (a.container, a.offset),
                            (b.container, b.offset),
                        )? <= 0
                    {
                        drop(dom);
                        set_selection_boundaries(
                            state,
                            if backward { b } else { a },
                            if backward { a } else { b },
                        )?;
                    }
                }
            }
        }
        "formatblock" => {
            let tag = value.trim_matches(|character| character == '<' || character == '>');
            let allowed = matches!(
                tag.to_ascii_lowercase().as_str(),
                "address"
                    | "article"
                    | "blockquote"
                    | "dd"
                    | "div"
                    | "dt"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "p"
                    | "pre"
            );
            if allowed {
                insert_editing_element(context, &tag.to_ascii_lowercase(), &[]).await?;
            }
        }
        "defaultparagraphseparator" => {
            let separator = value.to_ascii_lowercase();
            if matches!(separator.as_str(), "div" | "p") {
                *state.default_paragraph_separator.borrow_mut() = separator;
            }
        }
        _ => {}
    }
    Ok(())
}

// Native CSS parser access: never split declaration strings or retain V8 scope.
struct EditingFormatAdmission {
    association:Option<u64>,revision:u64,
    start:Option<editing_continuation::Marker>,end:Option<editing_continuation::Marker>,
}
impl EditingFormatAdmission {
    fn capture(context:&Context)->Result<Self,V8BindingError>{
        let state=&context.state;let association=state.selection_range.get();
        let (start,end)=if let Some(id)=association{
            let range=state.dom.borrow().live_range(LiveRangeId(id))?;
            let marker=|point:BoundaryPoint|context.marker(RangeRecord{start_container:point.container,start_offset:point.offset,end_container:point.container,end_offset:point.offset});
            (Some(marker(range.start)?),Some(marker(range.end)?))
        }else{(None,None)};
        Ok(Self{association,revision:state.selection_revision.get(),start,end})
    }
    fn current(&self,state:&V8BindingState)->Result<bool,V8BindingError>{
        if state.selection_range.get()!=self.association||state.selection_revision.get()!=self.revision{return Ok(false)}
        let Some(id)=self.association else{return Ok(true)};
        let live=state.dom.borrow().live_range(LiveRangeId(id))?;
        let a=self.start.as_ref().ok_or(V8BindingError::Allocation("format start marker"))?.range()?;
        let b=self.end.as_ref().ok_or(V8BindingError::Allocation("format end marker"))?.range()?;
        Ok(live.start.container==a.start_container&&live.start.offset==a.start_offset&&live.end.container==b.start_container&&live.end.offset==b.start_offset)
    }
}
fn editing_command_property(command:&str)->Option<&'static str>{
    match command {
        "bold"=>Some("font-weight"),"italic"=>Some("font-style"),
        "underline"|"strikethrough"=>Some("text-decoration-line"),
        "subscript"|"superscript"=>Some("vertical-align"),
        "fontname"=>Some("font-family"),"fontsize"=>Some("font-size"),
        "forecolor"=>Some("color"),"backcolor"|"hilitecolor"=>Some("background-color"),_=>None,
    }
}
fn editing_specified_command_value(state:&V8BindingState,node:NodeId,command:&str)->Result<Option<String>,V8BindingError>{
    if matches!(command,"subscript"|"superscript"){
        let dom=state.dom.borrow();let value=if dom.is_element(node,"http://www.w3.org/1999/xhtml","sup"){Some("superscript")}
            else if dom.is_element(node,"http://www.w3.org/1999/xhtml","sub"){Some("subscript")}else{None};
        return Ok(value.map(str::to_owned))
    }
    if matches!(command,"underline"|"strikethrough") {
        let parsed=editing_parsed_declaration(state,node,"text-decoration-line")?.value;
        if !parsed.is_empty() {
            let token=if command=="underline"{"underline"}else{"line-through"};
            return Ok(parsed.split_ascii_whitespace().any(|value|value==token).then(||token.to_owned()));
        }
    }
    if matches!(command,"backcolor"|"hilitecolor") && state.layout.borrow().styles.get(&node_key(node)).and_then(|s|s.get("display")).is_none_or(|v|v!="inline"){return Ok(None)}
    if let Some(property)=editing_command_property(command){
        let parsed=editing_parsed_declaration(state,node,property)?;
        if !parsed.value.is_empty(){return Ok(Some(parsed.value))}
    }
    let dom=state.dom.borrow();let record=dom.arena().get(node)?;
    let NodeKind::Element{name}=&record.kind else{return Ok(None)};
    if name.ns.as_ref()!="http://www.w3.org/1999/xhtml"{return Ok(None)}
    if command=="fontsize"&&name.local.as_ref()=="font" {
        return Ok(record.attributes.get_null("size")
            .and_then(|value|oryn_layout::html_font_size_hint_keyword(value)).map(str::to_owned));
    }
    let value=match (command,name.local.as_ref()){
        ("bold","b"|"strong")=>Some("bold"),("italic","i"|"em")=>Some("italic"),
        ("underline","u")=>Some("underline"),("strikethrough","s"|"strike")=>Some("line-through"),
        ("subscript","sub")=>Some("sub"),("superscript","sup")=>Some("super"),
        ("createlink"|"unlink","a")=>record.attributes.get_null("href").map(String::as_str),
        ("forecolor","font")=>record.attributes.get_null("color").map(String::as_str),
        ("fontname","font")=>record.attributes.get_null("face").map(String::as_str),
        ("fontsize","font")=>record.attributes.get_null("size").map(String::as_str),_=>None,
    };
    Ok(value.map(str::to_owned))
}

fn editing_parsed_declaration(
    state: &V8BindingState, node: NodeId, property: &str,
) -> Result<oryn_layout::cssom::DeclarationView,V8BindingError> {
    let base=state.native_document_base_url(true)?;
    let dom=state.dom.borrow();
    let record=dom.arena().get(node)?;
    let document=dom.arena().node_document(node)?.ok_or(V8BindingError::Allocation("editing declaration document"))?;
    if document!=state.document_root(){return Err(V8BindingError::Unsupported{interface:"Document".into(),member:"AbortError".into()})}
    let quirks=dom.arena().get(document)?.document_metadata.as_ref().is_some_and(|m|m.quirks);
    let css=record.attributes.get_null("style").map_or("",String::as_str);
    // Parse-only access does not resolve or fetch URLs; effective values are
    // supplied by the owning native layout, never this declaration reader.
    oryn_layout::cssom::read_declaration(css,property,&base,quirks).map_err(V8BindingError::EventCallback)
}

async fn editing_replace_tag_preserving_attributes(
    context:&Context,node:NodeId,tag:&str,
) -> Result<Option<NodeId>,V8BindingError> {
    let state=&context.state;
    let association=state.selection_range.get();let revision=state.selection_revision.get();
    let admission=EditingFormatAdmission::capture(context)?;
    let (parent,owner,attributes)={
        let dom=state.dom.borrow();
        let Some(parent)=dom.parent(node)? else{return Ok(None)};
        (parent,editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?,dom.arena().get(node)?.attributes.clone())
    };
    if !owner.is_some_and(|host|host!=node){return Ok(None)}
    let _node=context.lease(node)?;let _parent=context.lease(parent)?;
    let replacement=state.dom.borrow_mut().create_element(tag);
    let _replacement=context.lease(replacement)?;
    let admitted=||->Result<bool,V8BindingError>{
        let dom=state.dom.borrow();
        Ok(admission.current(state)? && state.selection_range.get()==association && state.selection_revision.get()==revision
            && dom.parent(node)?==Some(parent)
            && dom.parent(replacement)?==Some(parent)
            && element_local_name(&dom,replacement)==Some(tag)
            && editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?==owner)
    };
    context.append(parent,replacement,Some(node)).await?;
    if !admitted()?{
        if state.dom.borrow().parent(replacement)?==Some(parent) && light_children(&state.dom.borrow(),replacement)?.is_empty(){context.remove(replacement).await?;}
        return Ok(None)
    }
    for (index,attribute) in attributes.records().enumerate(){
        context.attribute(replacement,attribute.namespace.clone(),attribute.name.clone(),attribute.value.clone()).await?;
        if !admitted()?{
            if state.dom.borrow().parent(replacement)?==Some(parent)&&light_children(&state.dom.borrow(),replacement)?.is_empty(){context.remove(replacement).await?;}
            return Ok(None)
        }
        {let dom=state.dom.borrow();let current=&dom.arena().get(replacement)?.attributes;
            if current.records().count()!=index+1
                || attributes.records().take(index+1).any(|expected|!current.records().any(|actual|
                    actual.namespace==expected.namespace&&actual.name==expected.name&&actual.value==expected.value)) {
                return Ok(None)
            }
        }
    }
    let child_ids={let dom=state.dom.borrow();light_children(&dom,node)?};
    for child in child_ids {
        if !admitted()?{return Ok(None)}
        {let dom=state.dom.borrow();let current=&dom.arena().get(replacement)?.attributes;
            if current.records().count()!=attributes.records().count()
                || attributes.records().any(|expected|!current.records().any(|actual|
                    actual.namespace==expected.namespace&&actual.name==expected.name&&actual.value==expected.value)) {
                return Ok(None)
            }
        }
        if state.dom.borrow().parent(child)?!=Some(node){continue}
        let offset=boundary_length(&state.dom.borrow(),replacement)?;
        context.move_preserving_ranges(child,replacement,offset).await?;
    }
    if !admitted()?{return Ok(None)}
    if state.dom.borrow().arena().get(replacement)?.attributes!=attributes{return Ok(None)}
    // Child moves transfer old-parent offsets through the shared all-range
    // move algorithm. An empty old container has no such move, so transfer its
    // surviving zero boundaries before ordinary removal can collapse them.
    if light_children(&state.dom.borrow(),node)?.is_empty() {
        synchronize_live_ranges(state)?;
        let ranges=state.ranges.borrow().iter()
            .filter(|(id,_)|!state.static_ranges.borrow().contains(id))
            .filter(|(_,range)|range.start_container==node||range.end_container==node)
            .map(|(id,range)|(*id,range.clone())).collect::<Vec<_>>();
        for (id,expected) in ranges {
            let mut dom=state.dom.borrow_mut();
            let current=dom.live_range(LiveRangeId(id))?;
            if current.start.container!=expected.start_container||current.start.offset!=expected.start_offset
                ||current.end.container!=expected.end_container||current.end.offset!=expected.end_offset {continue}
            let start=if current.start.container==node{BoundaryPoint{container:replacement,offset:current.start.offset}}else{current.start};
            let end=if current.end.container==node{BoundaryPoint{container:replacement,offset:current.end.offset}}else{current.end};
            dom.set_live_range(LiveRangeId(id),start,end)?;
        }
        synchronize_live_ranges(state)?;
    }
    context.remove(node).await?;
    synchronize_live_ranges(state)?;
    Ok(Some(replacement))
}

async fn editing_clear_command_value(context:&Context,node:NodeId,command:&str)->Result<(),V8BindingError>{
    let state=&context.state;
    if editing_specified_command_value(state,node,command)?.is_none(){return Ok(())}
    let (owner,attributes,tag)={let dom=state.dom.borrow();
        (editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?,dom.arena().get(node)?.attributes.clone(),element_local_name(&dom,node).map(str::to_owned))};
    if !owner.is_some_and(|host|host!=node){return Ok(())}
    let association=state.selection_range.get();let revision=state.selection_revision.get();
    let admission=EditingFormatAdmission::capture(context)?;
    let admitted=||->Result<bool,V8BindingError>{Ok(admission.current(state)? && state.selection_range.get()==association && state.selection_revision.get()==revision
        && editing_host_for_node(&state.dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())?==owner)};
    let parsed=editing_parsed_declaration(state,node,"")?;
    let html_element=matches!(&state.dom.borrow().arena().get(node)?.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml");
    let simple_tag=html_element && matches!(tag.as_deref(),Some("a"|"b"|"em"|"font"|"i"|"s"|"span"|"strike"|"strong"|"sub"|"sup"|"u"));
    // Normative simple-modifiable classification belongs to the element,
    // not the currently requested command. I with font-weight is NOT simple:
    // unwrapping it while clearing bold would erase unrelated italic semantics.
    let style_property=parsed.names.first().map(String::as_str);
    let single_style=parsed.names.len()==1 && match tag.as_deref(){
        Some("b"|"strong")=>style_property==Some("font-weight"),
        Some("i"|"em")=>style_property==Some("font-style"),
        Some("a"|"font"|"span")=>style_property!=Some("text-decoration") && style_property!=Some("text-decoration-line"),
        _=>false,
    };
    let decoration=editing_parsed_declaration(state,node,"text-decoration")?;
    let single_decoration=parsed.names.len()==1 && matches!(tag.as_deref(),Some("a"|"font"|"s"|"span"|"strike"|"u"))
        && matches!(style_property,Some("text-decoration"|"text-decoration-line"))
        && matches!(decoration.value.as_str(),"line-through"|"underline"|"overline"|"none");
    let simple_style=attributes.len()==1 && attributes.get_null("style").is_some()
        && (parsed.names.is_empty() || single_style || single_decoration);
    let simple_attribute=attributes.len()==1 && match tag.as_deref(){Some("a")=>attributes.get_null("href").is_some(),Some("font")=>["color","face","size"].iter().any(|n|attributes.get_null(n).is_some()),_=>false};
    if simple_tag && (attributes.is_empty()||simple_style||simple_attribute){
        let parent=state.dom.borrow().parent(node)?.ok_or_else(hierarchy_request_error)?;
        let _parent=context.lease(parent)?;let _node=context.lease(node)?;
        let child_ids={let dom=state.dom.borrow();light_children(&dom,node)?};
        for child in child_ids {
            if !admitted()?||state.dom.borrow().parent(node)?!=Some(parent){return Ok(())}
            if state.dom.borrow().parent(child)?!=Some(node){continue}
            let (_,index)=relative_boundary(&state.dom.borrow(),node,false)?;
            context.move_preserving_ranges(child,parent,index).await?;
        }
        if admitted()?&&state.dom.borrow().parent(node)?==Some(parent){context.remove(node).await?;}
        return Ok(())
    }
    if let Some(property)=editing_command_property(command){
        if matches!(command,"underline"|"strikethrough"){
            let old=editing_parsed_declaration(state,node,"text-decoration-line")?.value;
            let remove=if command=="underline"{"underline"}else{"line-through"};
            let remaining=old.split_ascii_whitespace().filter(|value|*value!=remove).collect::<Vec<_>>().join(" ");
            context.style(node,property,remaining).await?;
        }else{context.style(node,property,"").await?;}
        if !admitted()?{return Ok(())}
        if editing_parsed_declaration(state,node,"")?.names.is_empty() && state.dom.borrow().arena().get(node)?.attributes.get_null("style").is_some(){
            context.remove_attribute(node,"style").await?;
            if !admitted()?{return Ok(())}
        }
    }
    let attribute=match (command,tag.as_deref()){("forecolor",Some("font"))=>Some("color"),("fontname",Some("font"))=>Some("face"),("fontsize",Some("font"))=>Some("size"),("createlink"|"unlink",Some("a"))=>Some("href"),_=>None};
    if let Some(attribute)=attribute{context.remove_attribute(node,attribute).await?;if !admitted()?{return Ok(())}}
    if editing_specified_command_value(state,node,command)?.is_some(){let _replacement=editing_replace_tag_preserving_attributes(context,node,"span").await?;}
    Ok(())
}

fn editing_effective_command_value(state:&V8BindingState,node:NodeId,command:&str)->Result<Option<String>,V8BindingError>{
    if matches!(command,"backcolor"|"hilitecolor") {
        let dom=state.dom.borrow();let layout=state.layout.borrow();
        let mut current=Some(node);let mut result=None;
        while let Some(node)=current {
            let record=dom.arena().get(node)?;
            if matches!(record.kind,NodeKind::Element{..}) {
                let value=layout.styles.get(&node_key(node)).and_then(|s|s.get("background-color")).cloned();
                let transparent=value.as_deref().is_none_or(|value|value=="transparent"||value.is_empty()||value=="rgba(0, 0, 0, 0)");
                result=value;
                if !transparent {break}
            }
            let parent=record.parent;
            if let Some(parent)=parent && !matches!(dom.arena().get(parent)?.kind,NodeKind::Element{..}){break}
            current=parent;
        }
        return Ok(result)
    }
    if matches!(command,"subscript"|"superscript"){
        let dom=state.dom.borrow();let layout=state.layout.borrow();let mut current=Some(node);let mut sub=false;let mut sup=false;
        while let Some(node)=current{
            let record=dom.arena().get(node)?;
            if matches!(record.kind,NodeKind::Element{..}) && layout.styles.get(&node_key(node)).and_then(|s|s.get("display")).is_none_or(|d|d!="inline"){break}
            if dom.is_element(node,"http://www.w3.org/1999/xhtml","sub"){sub=true}
            if dom.is_element(node,"http://www.w3.org/1999/xhtml","sup"){sup=true}
            current=record.parent;
        }
        return Ok(match (sub,sup){(true,true)=>Some("mixed"),(true,false)=>Some("subscript"),(false,true)=>Some("superscript"),_=>None}.map(str::to_owned))
    }
    if matches!(command,"underline"|"strikethrough"){
        let token=if command=="underline"{"underline"}else{"line-through"};
        let dom=state.dom.borrow();let layout=state.layout.borrow();let mut current=Some(node);
        while let Some(node)=current{
            if layout.styles.get(&node_key(node)).and_then(|s|s.get("text-decoration-line")).is_some_and(|v|v.split_ascii_whitespace().any(|v|v==token)){return Ok(Some(token.into()))}
            current=dom.parent(node)?;
        }
        return Ok(None)
    }
    let Some(property)=editing_command_property(command) else {
        let mut current=Some(node);
        while let Some(node)=current{
            if let Some(value)=editing_specified_command_value(state,node,command)?{return Ok(Some(value))}
            current=state.dom.borrow().parent(node)?;
        }
        return Ok(None)
    };
    let dom=state.dom.borrow();let mut current=Some(node);
    while let Some(node)=current{
        if let Some(value)=state.layout.borrow().styles.get(&node_key(node)).and_then(|s|s.get(property)){return Ok(Some(value.clone()))}
        current=dom.parent(node)?;
    }
    Ok(None)
}
fn editing_command_values_equivalent(command:&str,a:Option<&str>,b:Option<&str>)->bool{
    if a==b{return true}
    match command {
        "bold"=>matches!((a,b),(Some("bold"),Some("700"))|(Some("700"),Some("bold"))|(Some("normal"),Some("400"))|(Some("400"),Some("normal"))),
        "forecolor"|"backcolor"|"hilitecolor"=>match (a,b){
            (Some(a),Some(b))=>match (oryn_common::color::parse_color(a),oryn_common::color::parse_color(b)){
                (Some(a),Some(b))=>a==b,_=>false,
            },_=>false,
        },
        _=>false,
    }
}
// Force uses native CSSOM and mutation producers. Sibling coalescing and
// semantic-wrapper preference are supplied by the selected-run engine.
async fn editing_force_command_value(context:&Context,node:NodeId,command:&str,new_value:Option<&str>)->Result<(),V8BindingError>{
    let state=&context.state;
    let Some(value)=new_value else{return Ok(())};
    let admission=EditingFormatAdmission::capture(context)?;
    let association=state.selection_range.get();let revision=state.selection_revision.get();
    let endpoints=association.map(|id|state.dom.borrow().live_range(LiveRangeId(id))).transpose()?;
    context.layout().await?;
    if state.selection_range.get()!=association || state.selection_revision.get()!=revision
        || association.map(|id|state.dom.borrow().live_range(LiveRangeId(id))).transpose()?!=endpoints{return Ok(())}
    {let dom=state.dom.borrow();let layout=state.layout.borrow();let mut current=Some(node);
        while let Some(current_node)=current{
            if layout.styles.get(&node_key(current_node)).and_then(|s|s.get("display")).is_some_and(|d|d=="none"){return Ok(())}
            current=dom.parent(current_node)?;
        }
    }
    if editing_command_values_loosely_equivalent(context,command,editing_effective_command_value(state,node,command)?.as_deref(),Some(value)).await?{return Ok(())}
    if !admission.current(state)?{return Ok(())}
    if matches!(command,"createlink"|"unlink") {
        let mut parent=state.dom.borrow().parent(node)?;
        while let Some(ancestor)=parent {
            if !admission.current(state)?{return Ok(())}
            let next=state.dom.borrow().parent(ancestor)?;
            if state.dom.borrow().is_element(ancestor,"http://www.w3.org/1999/xhtml","a") {
                let _ancestor=context.lease(ancestor)?;
                if editing_replace_tag_preserving_attributes(context,ancestor,"span").await?.is_none(){return Ok(())}
            }
            parent=next;
        }
    }
    let property=editing_command_property(command);
    if property.is_none()&&command!="createlink"&&command!="unlink"{return Err(V8BindingError::Unsupported{interface:"Document".into(),member:"native formatting command value".into()})}
    let allowed={let dom=state.dom.borrow();match &dom.arena().get(node)?.kind{
        NodeKind::Document|NodeKind::DocumentFragment|NodeKind::DocumentType{..}=>false,
        NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"=>!matches!(name.local.as_ref(),
            "address"|"article"|"aside"|"blockquote"|"body"|"caption"|"center"|"col"|"colgroup"|"dd"|"details"|"dir"|"div"|"dl"|"dt"|"fieldset"|"figcaption"|"figure"|"footer"|"form"|"frame"|"frameset"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"head"|"header"|"hgroup"|"hr"|"html"|"li"|"listing"|"menu"|"nav"|"ol"|"p"|"plaintext"|"pre"|"section"|"summary"|"table"|"tbody"|"td"|"tfoot"|"th"|"thead"|"tr"|"ul"|"xmp"),_=>true}};
    if !allowed {
        // forceValue propagation recursively preserves the inherited value
        // when the block cannot be an inline wrapper child. This operation is
        // never the selected-node eligibility predicate: selection application
        // supplies only editable candidates, while pushDown supplies siblings.
        let owner=editing_host_for_node(&state.dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())?;
        let parent=state.dom.borrow().parent(node)?;
        let _node=context.lease(node)?;
        let child_ids={let dom=state.dom.borrow();light_children(&dom,node)?};
        let children=child_ids.into_iter().map(|node|context.lease(node)).collect::<Result<Vec<_>,_>>()?;
        for child in children{let (dom,child)=child.current();if !Rc::ptr_eq(&dom,&state.dom){return Ok(())}
            if !admission.current(state)?{return Ok(())}
            if state.dom.borrow().parent(node)?!=parent || editing_host_for_node(&state.dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())?!=owner{return Ok(())}
            if state.dom.borrow().parent(child)?!=Some(node){continue}
            if let Some(specified)=editing_specified_command_value(state,child,command)? && !editing_command_values_equivalent(command,Some(&specified),Some(value)){continue}
            Box::pin(editing_force_command_value(context,child,command,Some(value))).await?;
        }
        return Ok(())
    }
    let (parent,owner)={let dom=state.dom.borrow();(dom.parent(node)?,editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?)};
    let Some(parent)=parent else{return Ok(())};
    let expected_parent_owner=editing_host_for_node(&state.dom.borrow(),Some(parent),state.design_mode_enabled(),state.document_root())?;
    let admitted=||->Result<bool,V8BindingError>{
        let dom=state.dom.borrow();
        Ok(admission.current(state)? && state.selection_range.get()==association && state.selection_revision.get()==revision
            && dom.parent(node)?==Some(parent)
            && editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?==owner
            && editing_host_for_node(&dom,Some(parent),state.design_mode_enabled(),state.document_root())?==expected_parent_owner)
    };
    // Selection admission excludes noneditable islands. Native push-down may
    // still restore their previous inherited value; disallowed block children
    // are processed by the same normative force-value recursion above.
    let _parent=context.lease(parent)?;let _node=context.lease(node)?;
    let legacy_size=if command=="fontsize"&&(!state.style_with_css.get()||value=="xxx-large") {
        match value{"x-small"=>Some("1"),"small"=>Some("2"),"medium"=>Some("3"),"large"=>Some("4"),"x-large"=>Some("5"),"xx-large"=>Some("6"),"xxx-large"=>Some("7"),_=>None}
    }else{None};
    let semantic=match command{
        "fontname" if !state.style_with_css.get()=>Some("font"),
        "fontsize" if legacy_size.is_some()=>Some("font"),
        "createlink"=>Some("a"),"subscript"|"superscript" if value=="subscript"=>Some("sub"),"subscript"|"superscript" if value=="superscript"=>Some("sup"),
        _ if !state.style_with_css.get()=>match (command,value){("bold","bold"|"700")=>Some("b"),("italic","italic")=>Some("i"),("underline","underline")=>Some("u"),("strikethrough","line-through")=>Some("s"),_=>None},_=>None};
    let wrapper=state.dom.borrow_mut().create_element(semantic.unwrap_or("span"));let _wrapper=context.lease(wrapper)?;
    if command=="createlink"{context.attribute(wrapper,None,"href",value).await?;}
    else if command=="fontname"&&semantic==Some("font"){context.attribute(wrapper,None,"face",value).await?;}
    else if let Some(size)=legacy_size{context.attribute(wrapper,None,"size",size).await?;}
    else if semantic.is_none() && let Some(property)=property{context.style(wrapper,property,value).await?;}
    if !admitted()?{return Ok(())}
    context.append(parent,wrapper,Some(node)).await?;
    if !admitted()?{
        if state.dom.borrow().parent(wrapper)?==Some(parent)&&light_children(&state.dom.borrow(),wrapper)?.is_empty(){context.remove(wrapper).await?;}
        return Ok(())
    }
    let position=boundary_length(&state.dom.borrow(),wrapper)?;
    context.move_preserving_ranges(node,wrapper,position).await?;
    Ok(())
}

async fn editing_push_down_command_value(context:&Context,node:NodeId,command:&str,new_value:Option<&str>)->Result<(),V8BindingError>{
    let state=&context.state;
    let admission=EditingFormatAdmission::capture(context)?;
    context.layout().await?;
    if !admission.current(state)?{return Ok(())}
    if editing_command_values_loosely_equivalent(context,command,editing_effective_command_value(state,node,command)?.as_deref(),new_value).await?{return Ok(())}
    if !admission.current(state)?{return Ok(())}
    let mut ancestors=Vec::new();let mut current=state.dom.borrow().parent(node)?;
    while let Some(ancestor)=current{
        let eligible={
            let dom=state.dom.borrow();
            let owner=editing_host_for_node(&dom,Some(ancestor),state.design_mode_enabled(),state.document_root())?;
            owner.is_some_and(|owner|owner!=ancestor)&&matches!(dom.arena().get(ancestor)?.kind,NodeKind::Element{..})
        };
        if !eligible{break}
        if editing_command_values_loosely_equivalent(context,command,editing_effective_command_value(state,ancestor,command)?.as_deref(),new_value).await?{break}
        if !admission.current(state)?{return Ok(())}
        ancestors.push(context.lease(ancestor)?);
        current=state.dom.borrow().parent(ancestor)?;
    }
    let Some(last)=ancestors.last()else{return Ok(())};
    let (_,last)=last.current();
    let propagated=editing_specified_command_value(state,last,command)?;
    if propagated.is_none() && new_value.is_some(){return Ok(())}
    let mut propagated=propagated;
    let parent=state.dom.borrow().parent(last)?;
    let Some(parent)=parent else{return Ok(())};
    if new_value.is_some() && !editing_command_values_loosely_equivalent(context,command,editing_effective_command_value(state,parent,command)?.as_deref(),new_value).await?{return Ok(())}
    if !admission.current(state)?{return Ok(())}
    while let Some(lease)=ancestors.pop(){
        if !admission.current(state)?{return Ok(())}
        let (dom,ancestor)=lease.current();
        if !Rc::ptr_eq(&dom,&state.dom){return Ok(())}
        let specified=editing_specified_command_value(state,ancestor,command)?;
        if let Some(specified)=specified.as_ref(){propagated=Some(specified.clone());}
        let child_ids={let dom=dom.borrow();light_children(&dom,ancestor)?};
        let children=child_ids.into_iter().map(|child|context.lease(child)).collect::<Result<Vec<_>,_>>()?;
        if specified.is_some(){editing_clear_command_value(context,ancestor,command).await?;}
        if !admission.current(state)?{return Ok(())}
        let next=ancestors.last().map(|lease|lease.current().1);
        for child in children {
            if !admission.current(state)?{return Ok(())}
            let (dom,child)=child.current();
            if !Rc::ptr_eq(&dom,&state.dom)||child==node||Some(child)==next{continue}
            if let Some(specified)=editing_specified_command_value(state,child,command)?
                && !editing_command_values_equivalent(command,Some(&specified),propagated.as_deref()){continue}
            editing_force_command_value(context,child,command,propagated.as_deref()).await?;
        }
    }
    Ok(())
}

async fn editing_command_values_loosely_equivalent(
    context:&Context,command:&str,a:Option<&str>,b:Option<&str>,
)->Result<bool,V8BindingError>{
    if editing_command_values_equivalent(command,a,b){return Ok(true)}
    if command!="fontsize" {return Ok(false)}
    fn keyword(value:&str)->Option<BindingLegacyFontKeyword>{
        use BindingLegacyFontKeyword as K;
        Some(match value{"x-small"=>K::XSmall,"small"=>K::Small,"medium"=>K::Medium,
            "large"=>K::Large,"x-large"=>K::XLarge,"xx-large"=>K::XXLarge,"xxx-large"=>K::XXXLarge,_=>return None})
    }
    let Some((keyword,pixels))=a.zip(b).and_then(|(a,b)|{
        keyword(a).map(|k|(k,b)).or_else(||keyword(b).map(|k|(k,a)))
    })else{return Ok(false)};
    // Both values here originate from the native CSSOM specified/effective
    // descriptors. Require an actual resolved pixel value, never map arbitrary
    // relative units or unsupported colors through guessed conversion.
    let Some(pixels)=pixels.strip_suffix("px").and_then(|n|n.parse::<f32>().ok())
        .filter(|n|n.is_finite()&&*n>=0.)else{return Ok(false)};
    let state=&context.state;
    let admission=EditingFormatAdmission::capture(context)?;
    let resolved=context.legacy_font(keyword).await?;
    if !admission.current(state)?{return Ok(false)}
    Ok(pixels==resolved.pixels)
}

// Draft integration helper: selected editable runs are wrapped in place, never
// extracted across a noneditable island. Caller must supply post-layout live
// range, original host and command attributes after endpoint normalization.
fn formatting_equivalent_boundary(dom:&DomRuntime,container:NodeId,offset:u32)->Result<(NodeId,u32),V8BindingError>{
    if matches!(dom.arena().get(container)?.kind,NodeKind::Text{..}){
        if offset==0{return relative_boundary(dom,container,false)}
        if offset==boundary_length(dom,container)?{return relative_boundary(dom,container,true)}
    }
    Ok((container,offset))
}
struct SelectedFormatOptions<'a> {
    command: &'a str,
    value: &'a str,
    tag: &'a str,
    attributes: &'a [(&'a str, &'a str)],
    planned_toggle: Option<bool>,
}
async fn format_selected_editable_runs(
    context: &Context, range: &RangeRecord, host: NodeId,
    options: SelectedFormatOptions<'_>,
) -> Result<(), V8BindingError> {
    let SelectedFormatOptions{command,value,tag,attributes,planned_toggle}=options;
    let state = &context.state;
    let revision = state.selection_revision.get();
    let association = state.selection_range.get();
    let backward = state.selection_backward.get();
    let start = context.marker(RangeRecord {
        start_container:range.start_container, start_offset:range.start_offset,
        end_container:range.start_container, end_offset:range.start_offset,
    })?;
    let end = context.marker(RangeRecord {
        start_container:range.end_container, start_offset:range.end_offset,
        end_container:range.end_container, end_offset:range.end_offset,
    })?;
    let admitted = || -> Result<bool,V8BindingError> {
        if state.selection_revision.get()!=revision || state.selection_range.get()!=association { return Ok(false); }
        let Some(id)=association else { return Ok(false); };
        let live=state.dom.borrow().live_range(LiveRangeId(id))?;
        let a=start.range()?;let b=end.range()?;
        Ok(live.start.container==a.start_container && live.start.offset==a.start_offset
            && live.end.container==b.start_container && live.end.offset==b.start_offset)
    };
    // Split end before start so same-text offsets remain valid. Context::split
    // uses native live-range mutation bookkeeping and callback custody.
    for (node, offset) in [(range.end_container,range.end_offset),(range.start_container,range.start_offset)] {
        let length = boundary_length(&state.dom.borrow(),node)?;
        if matches!(state.dom.borrow().arena().get(node)?.kind,NodeKind::Text{..}) && offset > 0 && offset < length {
            context.split(node,offset).await?;
        }
        if !admitted()? { return Ok(()); }
    }
    let mut a = start.range()?; let mut b = end.range()?;
    // splitText leaves a boundary at the old text's length attached to the
    // old text. Its equivalent parent boundary includes the selected tail.
    for point in [&mut a,&mut b] {
        let dom=state.dom.borrow();
        let (container,offset)=formatting_equivalent_boundary(&dom,point.start_container,point.start_offset)?;
        point.start_container=container;point.start_offset=offset;
    }
    // One postorder projection, not a subtree scan for every candidate.
    let mut owners=BTreeMap::new();let mut uniform=BTreeMap::new();
    let order=traversal_nodes(&state.dom.borrow(),host)?;
    for node in &order {
        let dom=state.dom.borrow();let record=dom.arena().get(*node)?;
        let inherited=record.parent.and_then(|parent|owners.get(&parent).copied()).flatten();
        let owner=if *node==host { Some(host) } else {
            match record.attributes.get_null("contenteditable").map(|v|v.to_ascii_lowercase()).as_deref() {
                Some("false")=>None,
                Some(""|"true"|"plaintext-only")=>inherited.or(Some(*node)),
                _=>inherited,
            }
        };
        owners.insert(*node,owner);
    }
    for node in order.iter().rev() {
        let owner=owners[node];
        let same=light_children(&state.dom.borrow(),*node)?.iter().all(|child|owners.get(child)==Some(&owner) && uniform.get(child)==Some(&true));
        uniform.insert(*node,same);
    }
    let mut selected = Vec::new();
    let mut pending = light_children(&state.dom.borrow(),host)?.into_iter().rev().collect::<Vec<_>>();
    while let Some(node) = pending.pop() {
        let dom = state.dom.borrow();
        let before = relative_boundary(&dom,node,false)?;
        let after = relative_boundary(&dom,node,true)?;
        let intersects = compare_boundary_points(&dom,(b.start_container,b.start_offset),before)? > 0
            && compare_boundary_points(&dom,(a.start_container,a.start_offset),after)? < 0;
        if !intersects { continue; }
        let owner = owners[&node];
        let text = matches!(dom.arena().get(node)?.kind,NodeKind::Text{..});
        let fully = (compare_boundary_points(&dom,(a.start_container,a.start_offset),before)? <= 0
            || (text && a.start_container==node && a.start_offset==0))
            && (compare_boundary_points(&dom,after,(b.start_container,b.start_offset))? <= 0
            || (text && b.start_container==node && b.start_offset==boundary_length(&dom,node)?));
        // Editing specification's allowed child of span check: HTML content
        // model, not a rendering/tag guess about which nodes happen to be bold.
        let allowed = match &dom.arena().get(node)?.kind {
            NodeKind::Document|NodeKind::DocumentFragment|NodeKind::DocumentType{..}=>false,
            NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" => !matches!(name.local.as_ref(),
                "address"|"article"|"aside"|"blockquote"|"body"|"caption"|"center"|"col"|"colgroup"|"dd"|"details"|"dir"|"div"|"dl"|"dt"|"fieldset"|"figcaption"|"figure"|"footer"|"form"|"frame"|"frameset"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"head"|"header"|"hgroup"|"hr"|"html"|"li"|"listing"|"menu"|"nav"|"ol"|"p"|"plaintext"|"pre"|"section"|"summary"|"table"|"tbody"|"td"|"tfoot"|"th"|"thead"|"tr"|"ul"|"xmp"),
            _=>true,
        };
        if fully && allowed && owner.is_some_and(|owner|owner!=node) && uniform[&node] {
            // Invisible selected subtrees do not acquire an applied value.
            let mut ancestor=Some(node);let mut hidden=false;
            while let Some(current)=ancestor {
                if state.layout.borrow().styles.get(&node_key(current)).and_then(|s|s.get("display")).is_some_and(|d|d=="none") { hidden=true;break; }
                if current==host { break; }
                ancestor=dom.parent(current)?;
            }
            if hidden { continue; }
            drop(dom);
            selected.push((context.lease(node)?,owner));
        } else {
            // Descend even through false islands: nested explicitly editable
            // hosts must receive their own per-host formatting traversal.
            pending.extend(light_children(&dom,node)?.into_iter().rev());
        }
    }
    let boolean=matches!(command,"bold"|"italic"|"underline"|"strikethrough"|"subscript"|"superscript");
    let activated=match command{"bold"=>"bold","italic"=>"italic","underline"=>"underline","strikethrough"=>"line-through","subscript"=>"subscript","superscript"=>"superscript",_=>""};
    let mut toggle_off=boolean && !selected.is_empty();
    for (lease,_) in &selected{
        let (_,node)=lease.current();
        if !editing_command_values_equivalent(command,editing_effective_command_value(state,node,command)?.as_deref(),Some(activated)){toggle_off=false;break}
    }
    if let Some(planned)=planned_toggle { toggle_off=planned; }
    else {
        if command=="createlink" {
            let mut ancestors=BTreeSet::new();
            for (lease,_) in &selected {
                let (_,node)=lease.current();
                let mut parent=state.dom.borrow().parent(node)?;
                while let Some(node)=parent {
                    let dom=state.dom.borrow();
                    let owner=editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?;
                    if owner.is_some_and(|owner|owner!=node)
                        && dom.is_element(node,"http://www.w3.org/1999/xhtml","a")
                        && dom.arena().get(node)?.attributes.get_null("href").is_some() {
                        ancestors.insert(node);
                    }
                    parent=dom.parent(node)?;
                }
            }
            let ancestors=order.iter().filter(|node|ancestors.contains(node)).map(|node|context.lease(*node)).collect::<Result<Vec<_>,_>>()?;
            for lease in ancestors {
                if !admitted()?{return Ok(())}
                let (_,node)=lease.current();
                let eligible={let dom=state.dom.borrow();
                    let owner=editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?;
                    let mut contains_selected=false;
                    for (lease,_) in &selected {
                        let (_,selected)=lease.current();
                        let live_a=start.range()?;let live_b=end.range()?;
                        let a=formatting_equivalent_boundary(&dom,live_a.start_container,live_a.start_offset)?;
                        let b=formatting_equivalent_boundary(&dom,live_b.start_container,live_b.start_offset)?;
                        let before=relative_boundary(&dom,selected,false)?;
                        let after=relative_boundary(&dom,selected,true)?;
                        if compare_boundary_points(&dom,a,before)?>0 || compare_boundary_points(&dom,after,b)?>0 {continue}
                        let mut current=Some(selected);
                        while let Some(current_node)=current {
                            if current_node==node {contains_selected=true;break}
                            current=dom.parent(current_node)?;
                        }
                        if contains_selected {break}
                    }
                    owner.is_some_and(|owner|owner!=node)
                        && dom.is_element(node,"http://www.w3.org/1999/xhtml","a")
                        && dom.arena().get(node)?.attributes.get_null("href").is_some()
                        && contains_selected
                };
                if eligible {context.attribute(node,None,"href",value).await?;}
            }
            if !admitted()?{return Ok(())}
        }
        // Preserve the pre-clear command state, then discard every structural
        // plan after native clear callbacks. Re-enter once with live endpoints
        // so renamed/unwrapped elements cannot leave stale candidate ownership.
        let mut clear=Vec::new();
        for node in &order {
            let dom=state.dom.borrow();
            if !matches!(dom.arena().get(*node)?.kind,NodeKind::Element{..}) || !owners[node].is_some_and(|owner|owner!=*node){continue}
            let before=relative_boundary(&dom,*node,false)?;let after=relative_boundary(&dom,*node,true)?;
            if compare_boundary_points(&dom,(a.start_container,a.start_offset),before)?<=0 && compare_boundary_points(&dom,after,(b.start_container,b.start_offset))?<=0{
                drop(dom);
                clear.push((context.lease(*node)?,owners[node]));
            }
        }
        for (lease,expected_owner) in clear {
            if !admitted()? { return Ok(()); }
            let (dom,node)=lease.current();
            if !Rc::ptr_eq(&dom,&state.dom){return Ok(())}
            let still_contained={
                let dom=state.dom.borrow();let live_a=start.range()?;let live_b=end.range()?;
                let current_start=formatting_equivalent_boundary(&dom,live_a.start_container,live_a.start_offset)?;
                let current_end=formatting_equivalent_boundary(&dom,live_b.start_container,live_b.start_offset)?;
                let owner=editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?;
                if !dom.is_connected(node)?||owner!=expected_owner||!owner.is_some_and(|owner|owner!=node){false}else{
                    let before=relative_boundary(&dom,node,false)?;let after=relative_boundary(&dom,node,true)?;
                    compare_boundary_points(&dom,current_start,before)?<=0
                        && compare_boundary_points(&dom,after,current_end)?<=0
                }
            };
            if still_contained{editing_clear_command_value(context,node,command).await?;}
        }
        if !admitted()? { return Ok(()); }
        context.layout().await?;
        if !admitted()? { return Ok(()); }
        let Some(id)=association else{return Ok(())};
        let live:RangeRecord=state.dom.borrow().live_range(LiveRangeId(id))?.into();
        return Box::pin(format_selected_editable_runs(context,&live,host,SelectedFormatOptions{command,value,tag,attributes,planned_toggle:Some(toggle_off)})).await;
    }
    if toggle_off {
        // Apply command removal to formattable descendants rather than a
        // wrapper that clearValue may itself unwrap and retire.
        let mut targets=Vec::new();
        for (lease,_) in selected{
            let (_,node)=lease.current();
            let descendants={let dom=state.dom.borrow();traversal_nodes(&dom,node)?};
            for target in descendants {
                let dom=state.dom.borrow();
                let formattable=matches!(&dom.arena().get(target)?.kind,NodeKind::Text{..})
                    || matches!(element_local_name(&dom,target),Some("br"|"img"));
                let eligible=formattable && editing_host_for_node(&dom,Some(target),state.design_mode_enabled(),state.document_root())?.is_some();
                drop(dom);
                if eligible {targets.push(context.lease(target)?);}
            }
        }
        let neutral=if matches!(command,"underline"|"strikethrough"|"subscript"|"superscript"){None}else{Some("normal")};
        for lease in targets{
            if !admitted()?{return Ok(())}
            let (_,node)=lease.current();
            editing_push_down_command_value(context,node,command,neutral).await?;
            if !admitted()?{return Ok(())}
            if state.dom.borrow().arena().get(node).is_ok(){
                editing_force_command_value(context,node,command,neutral).await?;
            }
        }
        if admitted()?{
            let a=start.range()?;let b=end.range()?;
            let a=BoundaryPoint{container:a.start_container,offset:a.start_offset};let b=BoundaryPoint{container:b.start_container,offset:b.start_offset};
            set_selection_boundaries(state,if backward{b}else{a},if backward{a}else{b})?;
        }
        return Ok(())
    }
    let mut previous:Option<(NodeId,NodeId,Option<NodeId>)>=None;
    for (lease,expected_owner) in selected {
        let (dom,node) = lease.current();
        if !Rc::ptr_eq(&dom,&state.dom) || !admitted()? { return Ok(()); }
        if editing_host_for_node(&dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())? != expected_owner { continue; }
        let normalized;
        let target_value=if boolean{activated}else if command=="fontname"&&tag=="font" {value}
        else if command=="fontsize"&&tag=="font" {
            let size=attributes.iter().find(|(name,_)|*name=="size").map_or(value,|(_,value)|*value);
            oryn_layout::html_font_size_hint_keyword(size).unwrap_or(value)
        }else if let Some(property)=editing_command_property(command){
            let css=attributes.iter().find(|(name,_)|*name=="style").map_or("",|(_,value)|*value);
            let quirks=state.dom.borrow().arena().get(state.document_root())?.document_metadata.as_ref().is_some_and(|m|m.quirks);
            normalized=oryn_layout::cssom::read_declaration(css,property,&state.native_document_base_url(true)?,quirks).map_err(V8BindingError::EventCallback)?.value;
            if normalized.is_empty(){value}else{&normalized}
        }else{value};
        if editing_command_values_loosely_equivalent(context,command,editing_effective_command_value(state,node,command)?.as_deref(),Some(target_value)).await?{continue}
        if !admitted()?{return Ok(())}
        if editing_host_for_node(&dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())? != expected_owner {continue}
        editing_push_down_command_value(context,node,command,Some(target_value)).await?;
        if !admitted()?{return Ok(())}
        let parent = dom.borrow().parent(node)?.ok_or_else(hierarchy_request_error)?;
        let _parent=context.lease(parent)?;
        if let Some((prior_parent,wrapper,owner))=previous {
            let adjacent={let dom=dom.borrow();let children=dom.children(parent)?;
                children.iter().position(|n|*n==wrapper).is_some_and(|index|children.get(index+1)==Some(&node))};
            let wrapper_matches={let dom=dom.borrow();let record=dom.arena().get(wrapper)?;
                element_local_name(&dom,wrapper)==Some(tag)
                    && record.attributes.len()==attributes.len()
                    && attributes.iter().all(|(name,value)|record.attributes.get_null(name).is_some_and(|current|current==value))
                    && editing_host_for_node(&dom,Some(wrapper),state.design_mode_enabled(),state.document_root())?==expected_owner
                    && dom.parent(wrapper)?==Some(parent)};
            if prior_parent==parent && owner==expected_owner && adjacent && wrapper_matches {
                let index=boundary_length(&dom.borrow(),wrapper)?;
                context.move_preserving_ranges(node,wrapper,index).await?;
                continue;
            }
        }
        let wrapper = dom.borrow_mut().create_element(tag);
        let _wrapper=context.lease(wrapper)?;
        for (name,value) in attributes {
            context.attribute(wrapper,None,*name,*value).await?;
            if !admitted()? { return Ok(()); }
            if dom.borrow().parent(node)?!=Some(parent) || editing_host_for_node(&dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())?!=expected_owner{return Ok(())}
        }
        if !admitted()? { return Ok(()); }
        if dom.borrow().parent(node)?!=Some(parent) || !dom.borrow().is_connected(parent)?
            || editing_host_for_node(&dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())? != expected_owner { continue; }
        context.append(parent,wrapper,Some(node)).await?;
        if !admitted()? {
            if dom.borrow().parent(wrapper)?==Some(parent) && light_children(&dom.borrow(),wrapper)?.is_empty() { context.remove(wrapper).await?; }
            return Ok(());
        }
        if dom.borrow().parent(node)? != Some(parent)
            || editing_host_for_node(&dom.borrow(),Some(node),state.design_mode_enabled(),state.document_root())? != expected_owner {
            if dom.borrow().parent(wrapper)?==Some(parent) && light_children(&dom.borrow(),wrapper)?.is_empty() { context.remove(wrapper).await?; }
            continue;
        }
        let index=boundary_length(&dom.borrow(),wrapper)?;
        context.move_preserving_ranges(node,wrapper,index).await?;
        previous=Some((parent,wrapper,expected_owner));
    }
    if admitted()? {
        let a=start.range()?;let b=end.range()?;
        let a=BoundaryPoint{container:a.start_container,offset:a.start_offset};
        let b=BoundaryPoint{container:b.start_container,offset:b.start_offset};
        set_selection_boundaries(state,if backward {b}else{a},if backward {a}else{b})?;
    }
    Ok(())
}
pub(super) async fn apply_inline_format(
    context: &Context,
    command: &str,
    value: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let collapsed = state.selection_anchor.get().is_some()
        && state.selection_anchor.get() == state.selection_focus.get();
    let mut overrides = state.editing_overrides.borrow().clone();
    let css = state.style_with_css.get();
    let (mut tag, attribute) = match command {
        "bold" if css => ("span", Some(("style", "font-weight: bold"))),
        "bold" => ("b", None),
        "italic" if css => ("span", Some(("style", "font-style: italic"))),
        "italic" => ("i", None),
        "underline" if css => ("span", Some(("style", "text-decoration: underline"))),
        "underline" => ("u", None),
        "strikethrough" if css => ("span", Some(("style", "text-decoration: line-through"))),
        "strikethrough" => ("strike", None),
        "subscript" => ("sub", None),
        "superscript" => ("sup", None),
        "createlink" => ("a", Some(("href", value))),
        "fontname" => ("span", Some(("style", value))),
        "fontsize" => ("span", Some(("style", value))),
        "forecolor" => ("span", Some(("style", value))),
        "backcolor" | "hilitecolor" => ("span", Some(("style", value))),
        _ => ("span", None),
    };
    let owned;
    let attributes = match command {
        "fontname" => {
            if !css {
                tag="font";
                vec![("face",value)]
            } else {
                owned = format!("font-family: {value}");
                vec![("style", owned.as_str())]
            }
        }
        "fontsize" => {
            let size = match value {
                "1" => "x-small",
                "2" => "small",
                "3" => "medium",
                "4" => "large",
                "5" => "x-large",
                "6" => "xx-large",
                "7" => "xxx-large",
                other => other,
            };
            let legacy = match size {
                "x-small"=>Some("1"),"small"=>Some("2"),"medium"=>Some("3"),
                "large"=>Some("4"),"x-large"=>Some("5"),"xx-large"=>Some("6"),
                "xxx-large"=>Some("7"),_=>None,
            };
            if let Some(legacy)=legacy.filter(|_|!css||size=="xxx-large") {
                tag="font";
                owned=legacy.to_owned();
                vec![("size",owned.as_str())]
            } else {
                owned = format!("font-size: {size}");
                vec![("style", owned.as_str())]
            }
        }
        "forecolor" => {
            owned = format!("color: {value}");
            vec![("style", owned.as_str())]
        }
        "backcolor" | "hilitecolor" => {
            owned = format!("background-color: {value}");
            vec![("style", owned.as_str())]
        }
        _ => attribute.into_iter().collect(),
    };
    if !collapsed {
        let original_host=editing_host(state)?;
        let original_id=state.selection_range.get();
        let original_revision=state.selection_revision.get();
        let admission=EditingFormatAdmission::capture(context)?;
        context.layout().await?;
        synchronize_live_ranges(state)?;
        if !admission.current(state)? || state.selection_range.get()!=original_id || state.selection_revision.get()!=original_revision || editing_host(state)?!=original_host { return Ok(()); }
        let Some(host)=editing_host(state)? else { return Ok(()); };
        let Some(id)=state.selection_range.get() else { return Ok(()); };
        let range=state.ranges.borrow().get(&id).cloned().ok_or(V8BindingError::Allocation("Range record"))?;
        {
            format_selected_editable_runs(context,&range,host,SelectedFormatOptions{command,value,tag,attributes:&attributes,planned_toggle:None}).await?;
            return Ok(());
        }
    }
    insert_editing_element(context, tag, &attributes).await?;
    if collapsed {
        if let Some(value) = selection_dom_format_value(state, command) {
            let key = if command == "backcolor" {
                "hilitecolor"
            } else {
                command
            };
            overrides.insert(key.to_string(), value);
        }
        context.overrides(overrides).await?;
    }
    Ok(())
}

// Editing moves keep boundaries attached to moved content instead of applying
// the ordinary DOM removal rule that relocates them to the old parent.

pub(super) async fn apply_typing_overrides(
    context: &Context,
    overrides: &EditingOverrides,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    for (command, value) in &overrides.0 {
        if selection_dom_format_value(state, command).as_ref() != Some(value) {
            apply_inline_format(context, command, value).await?;
        }
    }
    Ok(())
}

// Shared identity-preserving inline-chain lift; callers publish final caret.
async fn lift_link_inline_chain(
    context: &Context,
    link: NodeId,
    chain: &[NodeId], // inner-to-outer existing single-child chain
    text: NodeId,
    original_host: NodeId,
    _after: bool,
) -> Result<bool, V8BindingError> {
    let state = &context.state;
    let Some(outer) = chain.last().copied() else { return Ok(true); };
    let inner = chain[0];
    let bytes = {
        let dom = state.dom.borrow();
        let mut bytes = chain.len().checked_add(1)
            .and_then(|count| count.checked_mul(std::mem::size_of::<(NodeId, oryn_dom::AttributeList)>()))
            .ok_or(V8BindingError::Allocation("link plan metadata overflow"))?;
        for node in std::iter::once(link).chain(chain.iter().copied()) {
            // AttributeList::clone copies its Vec of Arc pointers, not the
            // immutable ContentAttribute strings or records themselves.
            let pointers = dom.arena().get(node)?.attributes.len()
                .checked_mul(std::mem::size_of::<std::sync::Arc<oryn_dom::ContentAttribute>>())
                .ok_or(V8BindingError::Allocation("link attribute pointers overflow"))?;
            bytes = bytes.checked_add(pointers).ok_or(V8BindingError::Allocation("link attributes overflow"))?;
        }
        bytes
    };
    let association = state.selection_range.get();
    let admission_revision = state.selection_revision.get();
    let boundary_revision = state.dom.borrow().boundary_revision();
    let mutation = state.dom.borrow().mutation_cursor();
    let _allocation = context.retain_metadata(bytes).await?;
    synchronize_selection_range(state)?;
    if state.selection_range.get() != association || state.selection_revision.get() != admission_revision
        || state.dom.borrow().boundary_revision() != boundary_revision
        || state.dom.borrow().mutation_cursor() != mutation { return Ok(false); }
    let (parent, before, attributes) = {
        let dom = state.dom.borrow();
        let Some(parent) = dom.parent(link)? else { return Ok(false); };
        let mut attributes = Vec::new();
        attributes.try_reserve_exact(chain.len() + 1).map_err(|_| V8BindingError::Allocation("link attribute records"))?;
        if attributes.capacity() != chain.len() + 1 { return Err(V8BindingError::Allocation("link attribute capacity overshoot")); }
        for node in std::iter::once(link).chain(chain.iter().copied()) {
            attributes.push((node, dom.arena().get(node)?.attributes.clone_exact_capacity()
                .ok_or(V8BindingError::Allocation("link attribute allocation capacity"))?));
        }
        (parent, relative_boundary(&dom, link, false)?.1, attributes)
    };
    let id = state.selection_range.get().ok_or(V8BindingError::Allocation("link deletion selection"))?;
    let revision = state.selection_revision.get();
    let range = state.ranges.borrow().get(&id).cloned().ok_or(V8BindingError::Allocation("link deletion range"))?;
    let marker = context.marker(range)?;
    for node in std::iter::once(link).chain(chain.iter().copied()).chain([text, parent]) { context.lease(node)?; }
    let current = || -> Result<bool, V8BindingError> {
        synchronize_selection_range(state)?;
        let live = state.selection_range.get().and_then(|id| state.ranges.borrow().get(&id).cloned());
        let expected = marker.range()?;
        if state.selection_revision.get() != revision || state.selection_range.get() != Some(id) || live.as_ref().is_none_or(|live|
            live.start_container != expected.start_container || live.start_offset != expected.start_offset
            || live.end_container != expected.end_container || live.end_offset != expected.end_offset) { return Ok(false); }
        let dom = state.dom.borrow();
        if !dom.is_connected(link)? || dom.parent(link)? != Some(parent)
            || editing_host_for_node(&dom, Some(link), state.design_mode_enabled(), state.document_root())? != Some(original_host)
            || editing_host(state)? != Some(original_host) { return Ok(false); }
        for (node, expected) in &attributes { if &dom.arena().get(*node)?.attributes != expected { return Ok(false); } }
        Ok(true)
    };
    if !current()? { return Ok(false); }
    {
        let dom = state.dom.borrow();
        let mut child = text;
        for node in chain {
            if dom.parent(child)? != Some(*node) || dom.children(*node)? != [child] { return Ok(false); }
            child = *node;
        }
        if dom.parent(outer)? != Some(link) || dom.children(link)? != [outer] { return Ok(false); }
    }
    context.move_preserving_ranges(outer, parent, before).await?;
    if !current()? { return Ok(false); }
    if state.dom.borrow().parent(outer)? != Some(parent)
        || state.dom.borrow().parent(text)? != Some(inner) { return Ok(false); }
    context.move_preserving_ranges(text, link, 0).await?;
    if !current()? { return Ok(false); }
    if state.dom.borrow().parent(outer)? != Some(parent)
        || state.dom.borrow().parent(text)? != Some(link) { return Ok(false); }
    // This final move changes the expected link parent. A fresh current guard
    // must explicitly accept the planned inner parent, never the old parent.
    context.move_preserving_ranges(link, inner, 0).await?;
    synchronize_selection_range(state)?;
    let dom = state.dom.borrow();
    if !dom.is_connected(link)? || dom.parent(link)? != Some(inner)
        || dom.parent(outer)? != Some(parent)
        || editing_host_for_node(&dom, Some(link), state.design_mode_enabled(), state.document_root())? != Some(original_host) { return Ok(false); }
    let live = state.selection_range.get().and_then(|id| state.ranges.borrow().get(&id).cloned());
    let expected = marker.range()?;
    if state.selection_revision.get() != revision || state.selection_range.get() != Some(id) || live.as_ref().is_none_or(|live|
        live.start_container != expected.start_container || live.start_offset != expected.start_offset
        || live.end_container != expected.end_container || live.end_offset != expected.end_offset) { return Ok(false); }
    for (node, expected) in &attributes { if &dom.arena().get(*node)?.attributes != expected { return Ok(false); } }
    let mut child = link;
    for node in chain {
        if dom.parent(child)? != Some(*node) || dom.children(*node)? != [child] { return Ok(false); }
        child = *node;
    }
    Ok(true)
}

pub(super) async fn delete_selection_direction(
    context: &Context,
    forward: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        if join_rendered_editing_boundary(context, &range, forward).await? {
            return Ok(());
        }
let Some(range) = state.selection_range.get()
    .and_then(|id| state.ranges.borrow().get(&id).cloned())
else { return Ok(()); };
        // Selecting only the boundary between adjacent paragraphs has the
        // same line-joining behavior as deleting at either collapsed endpoint.
        if let Some(host) = editing_host(state)? {
            let left = nearest_rendered_editing_block(state, range.start_container, host)?;
            let right = nearest_rendered_editing_block(state, range.end_container, host)?;
            let boundary_only = {
                let dom = state.dom.borrow();
                left != right
                    && dom.parent(left)? == dom.parent(right)?
                    && editing_point_is_at_block_edge(
                        &dom,
                        BoundaryPoint {
                            container: range.start_container,
                            offset: range.start_offset,
                        },
                        left,
                        true,
                    )?
                    && editing_point_is_at_block_edge(
                        &dom,
                        BoundaryPoint {
                            container: range.end_container,
                            offset: range.end_offset,
                        },
                        right,
                        false,
                    )?
                    && dom.parent(left)?.is_some_and(|parent| {
                        light_children(&dom, parent).is_ok_and(|children| {
                            children
                                .iter()
                                .skip_while(|node| **node != left)
                                .skip(1)
                                .find(|node| !is_ignored_editing_sibling(&dom, **node))
                                == Some(&right)
                        })
                    })
            };
            if boundary_only
                && merge_adjacent_editing_block(context, range.start_container, true).await?
            {
                return Ok(());
            }
        }
        let preserve_nested_caret =
            !forward && selection_ends_at_nested_block_start(state, &range)?;
        // Explicit deletion differs from direct replacement: deleting an
        // anchor's first/last text character leaves typing outside that link.
        // Retain the link, not a stale text length, across native mutation.
        let original_host = editing_host(state)?;
        let association = state.selection_range.get();
        let revision = state.selection_revision.get();
        let boundary = state.dom.borrow().boundary_revision();
        let mutation = state.dom.borrow().mutation_cursor();
        let capacity = {
            let dom = state.dom.borrow(); let mut node = Some(range.start_container); let mut count = 0usize;
            while let Some(current) = node {
                count = count.checked_add(1).ok_or(V8BindingError::Allocation("link ancestor overflow"))?;
                if count > 16384 { return Err(V8BindingError::Allocation("link ancestor capacity")); }
                node = dom.parent(current)?;
            }
            count
        };
        let _metadata = context.retain_metadata(capacity.checked_mul(std::mem::size_of::<NodeId>())
            .ok_or(V8BindingError::Allocation("link edge plan overflow"))?).await?;
        synchronize_selection_range(state)?;
        if state.selection_range.get() != association || state.selection_revision.get() != revision
            || state.dom.borrow().boundary_revision() != boundary
            || state.dom.borrow().mutation_cursor() != mutation { return Ok(()); }
        let link_edge = {
            let dom = state.dom.borrow();
            if range.start_container == range.end_container {
                let mut node = range.start_container;
                let mut chain = Vec::new();
                chain.try_reserve_exact(capacity).map_err(|_|V8BindingError::Allocation("link edge chain"))?;
                if chain.capacity() != capacity { return Err(V8BindingError::Allocation("link edge capacity overshoot")); }
                let link = loop {
                    let Some(parent) = dom.parent(node)? else { break None };
                    if dom.children(parent)? != [node] { break None }
                    if dom.is_element(parent, "http://www.w3.org/1999/xhtml", "a") { break Some(parent) }
                    if Some(parent) == original_host || is_editing_block(&dom, parent)
                        || editing_host_for_node(&dom, Some(parent), state.design_mode_enabled(), state.document_root())? != original_host { break None }
                    chain.push(parent); node = parent;
                };
                if let Some(link) = link {
                    if Some(link) != original_host
                        && dom.arena().get(link)?.attributes.get_null("href").is_some()
                        && matches!(dom.arena().get(range.start_container)?.kind, NodeKind::Text { .. }) {
                        let edge = if range.start_offset == 0 { Some(false) }
                            else if range.end_offset == boundary_length(&dom, range.end_container)? { Some(true) }
                            else { None };
                        match edge { Some(after) => Some((link, after, dom.parent(link)?, chain)), None => None }
                    } else { None }
                } else { None }
            } else { None }
        };
        if let Some((link, _, _, chain)) = &link_edge {
            context.lease(*link)?;
            for node in chain { context.lease(*node)?; }
        }
        replace_selection_with_text(context, "").await?;
        if let Some((link, after, parent, chain)) = link_edge {
            let point = {
                let dom = state.dom.borrow();
                if parent.is_some() && dom.is_connected(link)? && dom.parent(link)? == parent
                    && state.selection_range.get() == association && state.selection_revision.get() == revision
                    && editing_host_for_node(&dom, Some(link), state.design_mode_enabled(), state.document_root())?
                        == original_host && editing_host(state)? == original_host {
                    let current = state.selection_range.get().and_then(|id| state.ranges.borrow().get(&id).cloned());
                    if current.as_ref().is_some_and(|current|
                        current.start_container == range.start_container
                        && current.end_container == range.start_container
                        && current.start_offset == range.start_offset
                        && current.end_offset == range.start_offset) {
                        let (container, offset) = relative_boundary(&dom, link, after)?;
                        Some(BoundaryPoint { container, offset })
                    } else { None }
                } else { None }
            };
            if let Some(point) = point {
                if chain.is_empty() { set_selection_boundaries(state, point, point)?; }
                else if let Some(host) = original_host {
                    if !lift_link_inline_chain(context, link, &chain, range.start_container, host, after).await? { return Ok(()); }
                    if state.selection_range.get() == association && state.selection_revision.get() == revision
                        && state.dom.borrow().is_connected(link)? && editing_host(state)? == original_host {
                        let (container, offset) = relative_boundary(&state.dom.borrow(), link, after)?;
                        let point = BoundaryPoint { container, offset };
                        set_selection_boundaries(state, point, point)?;
                    }
                }
            }
        }
        if preserve_nested_caret && state.dom.borrow().arena().get(range.end_container).is_ok() {
            let point = BoundaryPoint {
                container: range.end_container,
                offset: 0,
            };
            set_selection_boundaries(state, point, point)?;
        }
        return Ok(());
    }
    let kind = state
        .dom
        .borrow()
        .arena()
        .get(range.start_container)?
        .kind
        .clone();
    if let NodeKind::Text { data } | NodeKind::CDataSection { data } = kind {
        let (start, end) = utf16_delete_span(&data, range.start_offset, forward);
        if start == end {
            if !forward && remove_line_before_nested_block(context, range.start_container).await? {
                let point = BoundaryPoint {
                    container: range.start_container,
                    offset: range.start_offset,
                };
                set_selection_boundaries(state, point, point)?;
                return Ok(());
            }
            if join_rendered_editing_boundary(context, &range, forward).await? {
                return Ok(());
            }
            return merge_adjacent_editing_block(context, range.start_container, forward)
                .await
                .map(|_| ());
        }
        context
            .text(range.start_container, start, end.saturating_sub(start), "")
            .await?;
        if forward
            && end == oryn_dom::dom_string::len(&data) as u32
            && move_caret_to_following_editing_block(
                state,
                range.start_container,
                range.start_offset,
            )?
        {
            return Ok(());
        }
        let point = BoundaryPoint {
            container: range.start_container,
            offset: start,
        };
        set_selection_boundaries(state, point, point)?;
    } else {
        if join_rendered_editing_boundary(context, &range, forward).await? {
            return Ok(());
        }
        let placeholder = node_is_placeholder_empty(&state.dom.borrow(), range.start_container)?;
        if placeholder
            && merge_adjacent_editing_block(context, range.start_container, forward).await?
        {
            return Ok(());
        }
        let adjacent = {
            let dom = state.dom.borrow();
            let children = dom.arena().get(range.start_container)?.children.clone();
            if forward {
                children.get(range.start_offset as usize).copied()
            } else {
                range
                    .start_offset
                    .checked_sub(1)
                    .and_then(|index| children.get(index as usize))
                    .copied()
            }
        };
        if let Some(adjacent) = adjacent {
            let adjacent_kind = state.dom.borrow().arena().get(adjacent)?.kind.clone();
            if let NodeKind::Text { data } | NodeKind::CDataSection { data } = adjacent_kind {
                let length = oryn_dom::dom_string::len(&data) as u32;
                let (start, end) =
                    utf16_delete_span(&data, if forward { 0 } else { length }, forward);
                context
                    .text(adjacent, start, end.saturating_sub(start), "")
                    .await?;
                let point = if forward {
                    BoundaryPoint {
                        container: range.start_container,
                        offset: range.start_offset,
                    }
                } else {
                    BoundaryPoint {
                        container: adjacent,
                        offset: start,
                    }
                };
                set_selection_boundaries(state, point, point)?;
            } else if matches!(
                element_local_name(&state.dom.borrow(), adjacent),
                Some("br")
            ) {
                let container = range.start_container;
                context.remove(adjacent).await?;
                if forward
                    && move_caret_to_following_editing_block(state, container, range.start_offset)?
                {
                    return Ok(());
                }
                if light_children(&state.dom.borrow(), container)?.is_empty()
                    && is_editing_block(&state.dom.borrow(), container)
                {
                    let br = state.dom.borrow_mut().create_element("br");
                    context.append(container, br, None).await?;
                }
                let point = BoundaryPoint {
                    container,
                    offset: (if forward {
                        range.start_offset
                    } else {
                        range.start_offset.saturating_sub(1)
                    })
                    .min(boundary_length(&state.dom.borrow(), container)?),
                };
                set_selection_boundaries(state, point, point)?;
            } else {
                let trailing_break = (!forward)
                    .then(|| {
                        light_children(&state.dom.borrow(), range.start_container)
                            .ok()?
                            .get(range.start_offset as usize)
                            .copied()
                    })
                    .flatten()
                    .filter(|node| element_local_name(&state.dom.borrow(), *node) == Some("br"));
                if let Some(trailing_break) = trailing_break {
                    context.remove(trailing_break).await?;
                    let preceding = light_children(&state.dom.borrow(), range.start_container)?
                        .get(range.start_offset.saturating_sub(1) as usize)
                        .copied();
                    let point = if let Some(preceding) = preceding {
                        if node_is_placeholder_empty(&state.dom.borrow(), preceding)? {
                            editing_placeholder_caret(&state.dom.borrow(), preceding)?
                        } else {
                            BoundaryPoint {
                                container: preceding,
                                offset: editing_caret_end(&state.dom.borrow(), preceding)?,
                            }
                        }
                    } else {
                        BoundaryPoint {
                            container: range.start_container,
                            offset: range.start_offset.saturating_sub(1),
                        }
                    };
                    set_selection_boundaries(state, point, point)?;
                } else {
                    merge_adjacent_editing_block(context, adjacent, forward).await?;
                }
            }
        } else {
            merge_adjacent_editing_block(context, range.start_container, forward).await?;
        }
    }
    Ok(())
}

// Editing decisions use the same resolved style snapshot as getComputedStyle.
// Binding-only embedders without a layout provider retain an inherited fallback.

struct ListFragmentCustody {
    start: editing_continuation::Marker,
    end: editing_continuation::Marker,
    first: NodeId,
    last: NodeId,
    published: bool,
}

pub(super) async fn insert_editing_element(
    context: &Context,
    tag: &str,
    attributes: &[(&str, &str)],
) -> Result<NodeId, V8BindingError> {
    Ok(
        insert_editing_element_inner(context, tag, attributes, false)
            .await?
            .0,
    )
}
async fn insert_editing_element_with_list_custody(
    context: &Context,
    tag: &str,
    attributes: &[(&str, &str)],
) -> Result<(NodeId, Option<ListFragmentCustody>), V8BindingError> {
    insert_editing_element_inner(context, tag, attributes, true).await
}
async fn insert_editing_element_inner(
    context: &Context,
    tag: &str,
    attributes: &[(&str, &str)],
    list_custody: bool,
) -> Result<(NodeId, Option<ListFragmentCustody>), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let selection_revision = state.selection_revision.get();
    let backward = state.selection_backward.get();
    let original_host = editing_host(state)?;
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let marker = context.marker(RangeRecord {
        start_container: range.start_container,
        start_offset: range.start_offset,
        end_container: range.start_container,
        end_offset: range.start_offset,
    })?;
    let fragment = editing_range::contents(context, &range, RangeContentsMode::Extract).await?;
    synchronize_live_ranges(state)?;
    let insertion = marker.range()?;
    drop(marker);
    let element = state.dom.borrow_mut().create_element(tag);
    for (name, value) in attributes {
        context.attribute(element, None, *name, *value).await?;
    }
    let fragment_children = state.dom.borrow().children(fragment)?.to_vec();
    let mut custody = if list_custody && !fragment_children.is_empty() {
        let endpoint = |first: bool| -> Result<RangeRecord, V8BindingError> {
            let node = *if first {
                fragment_children.first()
            } else {
                fragment_children.last()
            }
            .ok_or(V8BindingError::Allocation("list source endpoint"))?;
            let offset = if first {
                0
            } else {
                boundary_length(&state.dom.borrow(), node)?
            };
            Ok(RangeRecord {
                start_container: node,
                start_offset: offset,
                end_container: node,
                end_offset: offset,
            })
        };
        Some(ListFragmentCustody {
            start: context.marker(endpoint(true)?)?,
            end: context.marker(endpoint(false)?)?,
            first: *fragment_children
                .first()
                .ok_or(V8BindingError::Allocation("list first"))?,
            last: *fragment_children
                .last()
                .ok_or(V8BindingError::Allocation("list last"))?,
            published: false,
        })
    } else {
        None
    };
    for child in fragment_children {
        if let Some(custody) = &custody {
            context
                .append_with_endpoint_markers(element, child, &[&custody.start, &custody.end])
                .await?;
        } else {
            context.append(element, child, None).await?;
        }
    }
    context
        .insert_at_boundary(insertion.start_container, insertion.start_offset, element)
        .await?;
    let end = light_children(&state.dom.borrow(), element)?.len() as u32;
    // A callback-authored selection supersedes the command's original extent.
    // Without such a change retain the source selection's direction.
    if state.selection_revision.get() == selection_revision {
        context.lease(element)?;
        let valid = {
            let dom = state.dom.borrow();
            dom.is_connected(element)?
                && original_host.is_some()
                && editing_host_for_node(
                    &dom,
                    Some(element),
                    state.design_mode_enabled(),
                    state.document_root(),
                )? == original_host
        };
        if valid {
            let (start, end) = if let Some(source) = &custody {
                context.lease(source.first)?;
                context.lease(source.last)?;
                let a = source.start.range()?;
                let b = source.end.range()?;
                let dom = state.dom.borrow();
                let start = if boundary_length(&dom, source.first)? == 0 {
                    let (container, offset) = relative_boundary(&dom, source.first, false)?;
                    BoundaryPoint { container, offset }
                } else {
                    BoundaryPoint {
                        container: a.start_container,
                        offset: a.start_offset,
                    }
                };
                let end = if boundary_length(&dom, source.last)? == 0 {
                    let (container, offset) = relative_boundary(&dom, source.last, true)?;
                    BoundaryPoint { container, offset }
                } else {
                    BoundaryPoint {
                        container: b.start_container,
                        offset: b.start_offset,
                    }
                };
                if !dom.is_connected(start.container)?
                    || !dom.is_connected(end.container)?
                    || editing_host_for_node(
                        &dom,
                        Some(start.container),
                        state.design_mode_enabled(),
                        state.document_root(),
                    )? != original_host
                    || editing_host_for_node(
                        &dom,
                        Some(end.container),
                        state.design_mode_enabled(),
                        state.document_root(),
                    )? != original_host
                    || compare_boundary_points(
                        &dom,
                        (start.container, start.offset),
                        (end.container, end.offset),
                    )? > 0
                {
                    return Ok((element, custody));
                }
                (start, end)
            } else {
                (
                    BoundaryPoint {
                        container: element,
                        offset: 0,
                    },
                    BoundaryPoint {
                        container: element,
                        offset: end,
                    },
                )
            };
            set_selection_boundaries(
                state,
                if backward { end } else { start },
                if backward { start } else { end },
            )?;
            if let Some(source) = &mut custody {
                source.published = true;
            }
        }
    }
    Ok((element, custody))
}

pub(super) async fn insert_editing_line_break(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    replace_selection_with_text(context, "").await?;
    context.layout().await?;
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let whitespace = editing_white_space(state, range.start_container)?;
    // Preformatted HTML containers use an explicit break element for editing
    // commands, even though their text layout preserves literal newlines.
    let preformatted_container = {
        let dom = state.dom.borrow();
        ["pre", "listing"]
            .into_iter()
            .any(|tag| has_ancestor_element_before(&dom, range.start_container, tag, host))
    };
    let preserves = !preformatted_container
        && matches!(
            whitespace.as_str(),
            "pre" | "pre-wrap" | "pre-line" | "break-spaces"
        );
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    let needs_placeholder = !editing_line_has_following_content(state, point, host, &whitespace)?;
    if preserves {
        replace_selection_with_text(context, "\n").await?;
        let range = state
            .ranges
            .borrow()
            .get(&id)
            .cloned()
            .ok_or(V8BindingError::Allocation("selection range"))?;
        point = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
    } else {
        let br = {
            let mut dom = state.dom.borrow_mut();
            if matches!(
                dom.arena().get(point.container)?.kind,
                NodeKind::Text { .. } | NodeKind::CDataSection { .. }
            ) {
                let length = boundary_length(&dom, point.container)?;
                if point.offset == 0 || point.offset == length {
                    let (container, offset) = relative_boundary(
                        &dom,
                        point.container,
                        point.offset == length && length != 0,
                    )?;
                    point = BoundaryPoint { container, offset };
                }
            }
            dom.create_element("br")
        };
        context
            .insert_at_boundary(point.container, point.offset, br)
            .await?;
        let dom = state.dom.borrow_mut();
        let (container, offset) = relative_boundary(&dom, br, true)?;
        point = BoundaryPoint { container, offset };
    }
    if needs_placeholder {
        let br = state.dom.borrow_mut().create_element("br");
        context
            .insert_at_boundary(point.container, point.offset, br)
            .await?;
    }
    set_selection_boundaries(state, point, point)?;
    if !preserves{normalize_line_start_whitespace(context,host).await?;}
    Ok(())
}

// Pasted markup carries its own inline formatting. Split the containing inline
async fn normalize_line_start_whitespace(context:&Context,host:NodeId)->Result<(),V8BindingError>{
    let state=&context.state;
    context.layout().await?;
    let point=typing_selection_point(state)?;
    let mode=editing_white_space(state,point.container)?;
    if matches!(mode.as_str(),"pre"|"pre-wrap"|"break-spaces"){return Ok(())}
    context.line_start_spaces(host,point).await
}
// ancestors up to the rendered block, preserving the content on either side.

async fn project_collapsed_link_typing_edge(context: &Context) -> Result<bool, V8BindingError> {
    let state = &context.state;
    synchronize_selection_range(state)?;
    let Some(id) = state.selection_range.get() else { return Ok(true); };
    let range = state.ranges.borrow().get(&id).cloned().ok_or(V8BindingError::Allocation("link typing range"))?;
    if range.start_container != range.end_container || range.start_offset != range.end_offset { return Ok(true); }
    let Some(host) = editing_host(state)? else { return Ok(true); };
    let (after, capacity, mutation) = {
        let dom = state.dom.borrow();
        if !matches!(dom.arena().get(range.start_container)?.kind, NodeKind::Text { .. }) { return Ok(true); }
        let length = boundary_length(&dom, range.start_container)?;
        let after = if range.start_offset == 0 { false }
            else if range.start_offset == length { true } else { return Ok(true); };
        let mut node = Some(range.start_container); let mut capacity = 0usize;
        while let Some(current) = node {
            capacity = capacity.checked_add(1).ok_or(V8BindingError::Allocation("link typing ancestor overflow"))?;
            node = dom.parent(current)?;
        }
        (after, capacity, dom.mutation_cursor())
    };
    let revision = state.selection_revision.get();
    let boundary = state.dom.borrow().boundary_revision();
    let _metadata = context.retain_metadata(capacity.checked_mul(std::mem::size_of::<NodeId>())
        .ok_or(V8BindingError::Allocation("link typing metadata overflow"))?).await?;
    synchronize_selection_range(state)?;
    if state.selection_range.get() != Some(id) || state.selection_revision.get() != revision
        || state.dom.borrow().mutation_cursor() != mutation
        || state.dom.borrow().boundary_revision() != boundary || editing_host(state)? != Some(host) { return Ok(false); }
    let fresh: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
    if fresh.start_container != range.start_container || fresh.start_offset != range.start_offset
        || fresh.end_container != range.end_container || fresh.end_offset != range.end_offset { return Ok(false); }
    let (link, parent, chain) = {
        let dom = state.dom.borrow(); let mut node = range.start_container;
        let mut chain = Vec::new();
        chain.try_reserve_exact(capacity).map_err(|_|V8BindingError::Allocation("link typing chain"))?;
        if chain.capacity() != capacity { return Err(V8BindingError::Allocation("link typing capacity overshoot")); }
        loop {
            let Some(parent) = dom.parent(node)? else { return Ok(true); };
            if parent == host || dom.children(parent)? != [node] { return Ok(true); }
            if dom.is_element(parent, "http://www.w3.org/1999/xhtml", "a") {
                if dom.arena().get(parent)?.attributes.get_null("href").is_none() { return Ok(true); }
                let Some(grand) = dom.parent(parent)? else { return Ok(true); };
                break (parent, grand, chain);
            }
            if is_editing_block(&dom, parent)
                || editing_host_for_node(&dom, Some(parent), state.design_mode_enabled(), state.document_root())? != Some(host) { return Ok(true); }
            chain.push(parent); node = parent;
        }
    };
    context.lease(link)?; context.lease(parent)?;
    for node in &chain { context.lease(*node)?; }
    if !chain.is_empty()
        && !lift_link_inline_chain(context, link, &chain, range.start_container, host, after).await? { return Ok(false); }
    synchronize_selection_range(state)?;
    let dom = state.dom.borrow();
    if state.selection_range.get() != Some(id) || state.selection_revision.get() != revision
        || !dom.is_connected(link)? || editing_host(state)? != Some(host)
        || (chain.is_empty() && dom.parent(link)? != Some(parent)) { return Ok(false); }
    let (container, offset) = relative_boundary(&dom, link, after)?;
    drop(dom);
    let point = BoundaryPoint { container, offset };
    set_selection_boundaries(state, point, point)?;
    Ok(true)
}

pub(super) async fn insert_editing_text(
    context: &Context,
    text: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    if !text.is_empty() && !text.starts_with('\n') {
        if !project_collapsed_link_typing_edge(context).await? { return Ok(()); }
    }
    let overrides = state.editing_overrides.borrow().clone();
    let units = oryn_dom::dom_string::units(text);
    let mut lines = units
        .split(|unit| *unit == 10)
        .map(oryn_dom::dom_string::from_units);
    let first = lines.next().unwrap_or_default();
    replace_selection_with_text(context, "").await?;
    if !first.is_empty() {
        apply_typing_overrides(context, &overrides).await?;
        remove_editing_placeholder_before_typing(context, &first).await?;
        context.layout().await?;
        let prepared = prepare_typing_whitespace(state, &first)?;
        replace_selection_with_text(context, &prepared).await?;
    }
    if text.is_empty() {
        context.layout().await?;
        prepare_typing_whitespace(state, text)?;
    }
    for line in lines {
        split_editing_paragraph(context).await?;
        if !line.is_empty() {
            apply_typing_overrides(context, &overrides).await?;
            remove_editing_placeholder_before_typing(context, &line).await?;
            context.layout().await?;
            let prepared = prepare_typing_whitespace(state, &line)?;
            replace_selection_with_text(context, &prepared).await?;
        }
    }
    context.overrides(overrides).await?;
    Ok(())
}

// The inserted text will give this line a caret position, so remove its
// redundant placeholder first. Text insertion must be the final DOM mutation
// exposed to editors. Keep visible breaks and stay inside this editing block.

pub(super) async fn join_first_editing_line(
    context: &Context,
    destination: NodeId,
    source: NodeId,
) -> Result<BoundaryPoint, V8BindingError> {
    let _state: &V8BindingState = &context.state;
    join_first_editing_line_before(context, destination, source, None).await
}

pub(super) async fn join_first_editing_line_before(
    context: &Context,
    destination: NodeId,
    source: NodeId,
    before: Option<NodeId>,
) -> Result<BoundaryPoint, V8BindingError> {
    let state: &V8BindingState = &context.state;
    context.layout().await?;
    let destination_mode = editing_white_space(state, destination)?;
    let source_is_placeholder = matches!(
        state.dom.borrow().arena().get(source)?.kind,
        NodeKind::Element { .. }
    ) && node_is_placeholder_empty(&state.dom.borrow(), source)?;
    let mut line_root = source;
    loop {
        let first = light_children(&state.dom.borrow(), line_root)?
            .into_iter()
            .find(|node| !is_ignored_editing_sibling(&state.dom.borrow(), *node));
        let Some(first) = first else { break };
        if nearest_rendered_editing_block(state, first, source)? != first {
            break;
        }
        line_root = first;
    }
    let source_mode = editing_white_space(state, line_root)?;
    // Matching whitespace modes can retain the source's internal line breaks
    // in a single merged block. Different modes and nested source blocks use
    // the first-line join, preserving the remaining source paragraph's style.
    let host = editing_host(state)?.unwrap_or(destination);
    let merge_whole_block = before.is_none()
        && line_root == source
        && source_mode == destination_mode
        && nearest_rendered_editing_block(state, destination, host)? == destination
        && nearest_rendered_editing_block(state, source, host)? == source;
    let source_is_inline = matches!(
        state.dom.borrow().arena().get(line_root)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) || state
        .layout
        .borrow()
        .styles
        .get(&node_key(line_root))
        .and_then(|style| style.get("display"))
        .is_some_and(|display| display == "inline");
    // Semantic inline elements are part of the moved line, rather than its
    // paragraph container. A span used as a whitespace carrier is rebuilt below.
    let preserve_inline_shell = source_is_inline
        && matches!(
            state.dom.borrow().arena().get(line_root)?.kind,
            NodeKind::Element { .. }
        )
        && element_local_name(&state.dom.borrow(), line_root) != Some("span");
    let destination_absorbs_inline_whitespace = source_is_inline
        && editing_host(state)?.is_some_and(|host| {
            let dom = state.dom.borrow();
            ["pre", "listing"]
                .into_iter()
                .any(|tag| has_ancestor_element_before(&dom, destination, tag, host))
        });
    let source_has_author_whitespace = {
        let dom = state.dom.borrow();
        let style_node = if matches!(
            dom.arena().get(line_root)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            dom.parent(line_root)?.unwrap_or(line_root)
        } else {
            line_root
        };
        state
            .layout
            .borrow()
            .author_whitespace
            .contains(&node_key(style_node))
    };
    // Reuse a single inline container when that preserves its computed mode
    // without rewriting an existing inline declaration block.
    let reusable_inline = light_children(&state.dom.borrow(), line_root)?
        .first()
        .copied()
        .is_some_and(|node| {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            matches!(
                dom.arena().get(node).map(|n| &n.kind),
                Ok(NodeKind::Element { .. })
            ) && dom
                .attribute_in_null_namespace(node, "style")
                .ok()
                .flatten()
                .is_none()
                && layout.styles.get(&node_key(node)).is_some_and(|style| {
                    style
                        .get("display")
                        .is_some_and(|display| display == "inline")
                        && style.get("white-space") == Some(&source_mode)
                })
        });
    let mut end = BoundaryPoint {
        container: line_root,
        offset: boundary_length(&state.dom.borrow(), line_root)?,
    };
    let mut separator = None;
    let mut cleanup = Vec::new();
    let mut line_nodes = if matches!(
        state.dom.borrow().arena().get(line_root)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) {
        vec![line_root]
    } else {
        light_children(&state.dom.borrow(), line_root)?
    };
    line_nodes.reverse();
    while let Some(node) = line_nodes.pop() {
        let kind = state.dom.borrow().arena().get(node)?.kind.clone();
        if is_preserved_editing_boundary(&state.dom.borrow(), node) {
            let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
            end = BoundaryPoint { container, offset };
            break;
        }
        match kind {
            NodeKind::Element { .. } => {
                let hidden = state
                    .layout
                    .borrow()
                    .styles
                    .get(&node_key(node))
                    .and_then(|style| style.get("display"))
                    .is_some_and(|display| display == "none");
                if hidden || is_ignored_editing_sibling(&state.dom.borrow(), node) {
                    continue;
                }
                if !merge_whole_block && element_local_name(&state.dom.borrow(), node) == Some("br")
                {
                    let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
                    end = BoundaryPoint { container, offset };
                    separator = Some((node, false));
                    break;
                }
                context.layout().await?;
                {
                    let dom = state.dom.borrow();
                    if !dom.is_connected(node)? || !matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. })
                        || is_ignored_editing_sibling(&dom, node) || is_preserved_editing_boundary(&dom, node)
                        || state.layout.borrow().styles.get(&node_key(node)).and_then(|style| style.get("display")).is_some_and(|display| display == "none")
                    { continue; }
                    if !merge_whole_block && element_local_name(&dom, node) == Some("br") {
                        let (container, offset) = relative_boundary(&dom, node, false)?;
                        end = BoundaryPoint { container, offset };
                        separator = Some((node, false));
                        break;
                    }
                }
                if nearest_rendered_editing_block(state, node, line_root)? == node {
                    let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
                    end = BoundaryPoint { container, offset };
                    break;
                }
                line_nodes.extend(light_children(&state.dom.borrow(), node)?.into_iter().rev());
            }
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
                context.layout().await?;
                let data = match state.dom.borrow().arena().get(node)?.kind.clone() {
                    NodeKind::Text { data } | NodeKind::CDataSection { data } => data,
                    _ => continue,
                };
                let mode = editing_white_space(state, node)?;
                if !merge_whole_block
                    && matches!(
                        mode.as_str(),
                        "pre" | "pre-wrap" | "pre-line" | "break-spaces"
                    )
                    && let Some(index) = oryn_dom::dom_string::units(&data)
                        .iter()
                        .position(|unit| *unit == 10)
                {
                    end = BoundaryPoint {
                        container: node,
                        offset: index as u32,
                    };
                    separator = Some((node, true));
                    break;
                }
            }
            _ => {}
        }
    }
    // The paragraph's first line can end at a break immediately outside an
    // inline source container (including a bare text node).
    let following_break = if source_is_inline
        && !source_is_placeholder
        && separator.is_none()
        && end.container == line_root
        && end.offset == boundary_length(&state.dom.borrow(), line_root)?
    {
        let dom = state.dom.borrow();
        dom.parent(source)?
            .map(|parent| light_children(&dom, parent))
            .transpose()?
            .and_then(|children| {
                children
                    .windows(2)
                    .find(|pair| pair[0] == source)
                    .map(|pair| pair[1])
            })
            .filter(|node| element_local_name(&dom, *node) == Some("br"))
    } else {
        None
    };
    if let Some((node, _)) = separator {
        let mut current = Some(node);
        while let Some(node) = current {
            cleanup.push(node);
            if node == line_root {
                break;
            }
            current = state.dom.borrow().parent(node)?;
        }
    }
    let caret_offset = {
        let dom = state.dom.borrow();
        if let Some(before) = before {
            relative_boundary(&dom, before, false)?.1
        } else {
            editing_caret_end(&dom, destination)?
        }
    };
    let fragment = editing_range::contents(
        context,
        &RangeRecord {
            start_container: line_root,
            start_offset: 0,
            end_container: end.container,
            end_offset: end.offset,
        },
        RangeContentsMode::Extract,
    )
    .await?;
    if let Some((node, newline)) = separator {
        if newline {
            context.text(node, 0, 1, "").await?;
        } else {
            let has_parent = state.dom.borrow().parent(node)?.is_some();
            if has_parent {
                context.remove(node).await?;
            }
        }
    }
    for node in cleanup {
        if node == line_root {
            break;
        }
        let remove = {
            let dom = state.dom.borrow();
            let empty = match &dom.arena().get(node)?.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } => data.is_empty(),
                NodeKind::Element { .. } => dom.arena().get(node)?.children.is_empty(),
                _ => false,
            };
            empty && dom.parent(node)?.is_some()
        };
        if remove {
            context.remove(node).await?;
        }
    }
    if let Some(node) = following_break {
        let has_parent = state.dom.borrow().parent(node)?.is_some();
        if has_parent {
            context.remove(node).await?;
        }
    }
    if preserve_inline_shell && !source_is_placeholder {
        let children = light_children(&state.dom.borrow(), fragment)?;
        if !children.is_empty() {
            let shell = {
                let mut dom = state.dom.borrow_mut();
                if light_children(&dom, line_root)?.is_empty() {
                    // Reuse a wholly consumed element so listeners and identity survive.
                    line_root
                } else {
                    dom.clone_node(line_root, false)?
                }
            };
            for child in children {
                context.append(shell, child, None).await?;
            }
            context.append(fragment, shell, None).await?;
        }
    }
    let moving = if source_is_placeholder {
        Vec::new()
    } else {
        light_children(&state.dom.borrow(), fragment)?
    };
    if !moving.is_empty() {
        // A final placeholder is replaced by the joined line.
        let placeholder = {
            let dom = state.dom.borrow();
            if before.is_none() {
                light_children(&dom, destination)?
                    .last()
                    .copied()
                    .filter(|last| element_local_name(&dom, *last) == Some("br"))
            } else {
                None
            }
        };
        if let Some(last) = placeholder {
            context.remove(last).await?;
        }
        let target = if source_mode != destination_mode
            && !destination_absorbs_inline_whitespace
            && (source_has_author_whitespace || source_is_inline)
        {
            let reuse = reusable_inline
                && !preserve_inline_shell
                && moving.len() == 1
                && matches!(
                    state.dom.borrow().arena().get(moving[0])?.kind,
                    NodeKind::Element { .. }
                );
            if reuse {
                context
                    .attribute(
                        moving[0],
                        None,
                        "style",
                        format!("white-space: {source_mode};"),
                    )
                    .await?;
                destination
            } else {
                let span = state.dom.borrow_mut().create_element("span");
                context
                    .attribute(span, None, "style", format!("white-space: {source_mode};"))
                    .await?;
                context.append(destination, span, before).await?;
                span
            }
        } else {
            destination
        };
        for child in moving {
            context
                .append(
                    target,
                    child,
                    if target == destination { before } else { None },
                )
                .await?;
        }
    }
    let mut empty = line_root;
    loop {
        let parent = {
            let dom = state.dom.borrow();
            let is_empty = match &dom.arena().get(empty)?.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } => data.is_empty(),
                _ => light_children(&dom, empty)?.is_empty(),
            };
            if !is_empty {
                break;
            }
            dom.parent(empty)?
        };
        let Some(parent) = parent else { break };
        context.remove(empty).await?;
        if empty == source {
            break;
        }
        empty = parent;
    }
    let dom = state.dom.borrow();
    if node_is_placeholder_empty(&dom, destination)? {
        return editing_placeholder_caret(&dom, destination);
    }
    Ok(BoundaryPoint {
        container: destination,
        offset: caret_offset.min(boundary_length(&dom, destination)?),
    })
}

pub(super) async fn join_rendered_editing_boundary(
    context: &Context,
    range: &RangeRecord,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    if element_local_name(&state.dom.borrow(), host) == Some("html") {
        return Ok(false);
    }
    context.layout().await?;
    let selected =
        range.start_container != range.end_container || range.start_offset != range.end_offset;
    let start = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    let Some((left, right)) = rendered_editing_boundary(state, start, host, selected || forward)?
    else {
        return Ok(false);
    };
    if selected {
        let end = BoundaryPoint {
            container: range.end_container,
            offset: range.end_offset,
        };
        if rendered_editing_boundary(state, end, host, false)? != Some((left, right)) {
            return Ok(false);
        }
    }
    let parent = state
        .dom
        .borrow()
        .parent(left)?
        .ok_or(V8BindingError::Allocation("editing boundary parent"))?;
    let left_is_block = nearest_rendered_editing_block(state, left, host)? == left;
    let (mut destination, before) = if left_is_block {
        (left, None)
    } else {
        (parent, Some(right))
    };
    if left_is_block {
        loop {
            let children = light_children(&state.dom.borrow(), destination)?;
            let mut last = None;
            for child in children.into_iter().rev() {
                if !editing_join_ignored(state, child)? {
                    last = Some(child);
                    break;
                }
            }
            let Some(last) = last else {
                break;
            };
            if editing_join_structural_boundary(&state.dom.borrow(), last)
                || nearest_rendered_editing_block(state, last, host)? != last
            {
                break;
            }
            destination = last;
        }
    }
    let children = light_children(&state.dom.borrow(), parent)?;
    let left_index = children
        .iter()
        .position(|node| *node == left)
        .ok_or(V8BindingError::Allocation("editing left unit"))?;
    let right_index = children
        .iter()
        .position(|node| *node == right)
        .ok_or(V8BindingError::Allocation("editing right unit"))?;
    for node in &children[left_index + 1..right_index] {
        if matches!(
            state.dom.borrow().arena().get(*node)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            context.remove(*node).await?;
        }
    }
    let point = join_first_editing_line_before(context, destination, right, before).await?;
    set_selection_boundaries(state, point, point)?;
    Ok(true)
}

pub(super) async fn merge_adjacent_editing_block(
    context: &Context,
    container: NodeId,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    let block = nearest_editing_block(&state.dom.borrow(), container, host)?;
    if block == host {
        return merge_document_root_editing_boundary(context, container, host, forward).await;
    }
    let (block, parent, sibling, ignored) = {
        let dom = state.dom.borrow();
        let Some(parent) = dom.parent(block)? else {
            return Ok(false);
        };
        let children = light_children(&dom, parent)?;
        let Some(index) = children.iter().position(|node| *node == block) else {
            return Ok(false);
        };
        let sibling_index = if forward {
            children
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, candidate)| !is_ignored_editing_sibling(&dom, **candidate))
                .map(|(index, _)| index)
        } else {
            children[..index]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, candidate)| !is_ignored_editing_sibling(&dom, **candidate))
                .map(|(index, _)| index)
        };
        let sibling = sibling_index.map(|index| children[index]);
        let ignored = sibling_index.map_or_else(Vec::new, |sibling_index| {
            let range = if forward {
                index + 1..sibling_index
            } else {
                sibling_index + 1..index
            };
            children[range]
                .iter()
                .copied()
                .filter(|candidate| is_ignored_editing_sibling(&dom, *candidate))
                .collect()
        });
        (block, parent, sibling, ignored)
    };
    if let Some(sibling) = sibling
        && state.dom.borrow().arena().get(sibling).is_ok()
        && matches!(
            state.dom.borrow().arena().get(sibling)?.kind,
            NodeKind::Element { .. }
        )
        && is_mergeable_editing_sibling(&state.dom.borrow(), sibling)
    {
        let (destination, source) = if forward {
            (block, sibling)
        } else {
            (sibling, block)
        };
        for separator in ignored {
            let whitespace = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(separator).map(|record| &record.kind),
                    Ok(NodeKind::Text { data } | NodeKind::CDataSection { data }) if data.trim().is_empty())
            };
            if whitespace {
                context.remove(separator).await?;
            }
        }
        let point = join_first_editing_line(context, destination, source).await?;
        set_selection_boundaries(state, point, point)?;
        return Ok(true);
    }

    // An empty final nested list/list-item collapses back into its containing
    // editable item rather than leaving an unreachable empty sub-list.
    if node_is_placeholder_empty(&state.dom.borrow(), block)?
        && matches!(
            element_local_name(&state.dom.borrow(), block),
            Some("li" | "dt" | "dd")
        )
        && matches!(
            element_local_name(&state.dom.borrow(), parent),
            Some("ul" | "ol" | "dl")
        )
        && parent != host
    {
        let grand = state.dom.borrow().parent(parent)?.unwrap_or(host);
        let grand_tag = element_local_name(&state.dom.borrow(), grand).map(str::to_string);
        if matches!(grand_tag.as_deref(), Some("li" | "dt" | "dd")) {
            context.remove(parent).await?;
        } else {
            let replacement_tag = if grand_tag.as_deref() == Some("dl") {
                "dd"
            } else {
                "li"
            };
            let replacement =
                if element_local_name(&state.dom.borrow(), block) == Some(replacement_tag) {
                    block
                } else {
                    // Flatten into the item type required by the containing list.
                    let item = state.dom.borrow_mut().create_element(replacement_tag);
                    let children = light_children(&state.dom.borrow(), block)?;
                    for child in children {
                        context.append(item, child, None).await?;
                    }
                    item
                };
            context.append(grand, replacement, Some(parent)).await?;
            context.remove(parent).await?;
        }
        let point = BoundaryPoint {
            container: grand,
            offset: editing_caret_end(&state.dom.borrow(), grand)?,
        };
        set_selection_boundaries(state, point, point)?;
        return Ok(true);
    }
    merge_document_root_editing_boundary(context, container, host, forward).await
}

pub(super) async fn replace_selection_with_html(
    context: &Context,
    markup: &str,
    pasted_markup: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let overrides = state.editing_overrides.borrow().clone();
    replace_selection_with_text(context, "").await?;
    if pasted_markup { context.layout().await?; }
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let parsed = oryn_dom::html::parse_fragment_dom_string_with_context(
        markup,
        markup5ever::QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "div".into()),
        Vec::new(),
        false,
    );
    let document_children = parsed.dom.get(parsed.document)?.children.clone();
    let children = document_children
        .first()
        .and_then(|root| parsed.dom.get(*root).ok())
        .filter(|root| {
            matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html")
        })
        .map(|root| root.children.clone())
        .unwrap_or(document_children);
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    if children.is_empty() {
        return Ok(());
    }
    if pasted_markup {
        point = html_insertion_boundary(state, point)?;
    }
    let mut last_inserted = None;
    for child in children {
        let imported = state.dom.borrow_mut().import_subtree(&parsed.dom, child)?;
        context
            .insert_at_boundary(point.container, point.offset, imported)
            .await?;
        last_inserted = Some(imported);
        let (container, offset) = relative_boundary(&state.dom.borrow(), imported, true)?;
        point = BoundaryPoint { container, offset };
    }
    if pasted_markup && let Some(mut last) = last_inserted {
        let dom = state.dom.borrow();
        while let Some(child) = light_children(&dom, last)?.last().copied() {
            last = child;
        }
        if matches!(
            dom.arena().get(last)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            point = BoundaryPoint {
                container: last,
                offset: boundary_length(&dom, last)?,
            };
        }
    }
    set_selection_boundaries(state, point, point)?;
    context.overrides(overrides).await?;
    Ok(())
}

pub(super) async fn replace_selection_with_text(
    context: &Context,
    replacement: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    // Layout callbacks may retarget selection. Capture its authority only
    // after the existing native provider has published rendered block styles.
    if !replacement.is_empty() {
        context.layout().await?;
    }
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "no active selection".into(),
        })?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let affected_blocks = editing_host(state)?.and_then(|host| {
        let dom = state.dom.borrow();
        let start = nearest_editing_block(&dom, range.start_container, host).ok()?;
        let end = nearest_editing_block(&dom, range.end_container, host).ok()?;
        (start != end).then_some((start, end))
    });
    let has_selection = compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0;
    let preserve_nested_caret =
        has_selection && selection_ends_at_nested_block_start(state, &range)?;
    // A selection leaving an anchor must not inherit that anchor's href
    // when its replacement is inserted, including direct typing replacement.
    let crossed_link = if has_selection {
        let association = state.selection_range.get();
        let revision = state.selection_revision.get();
        let mutation = state.dom.borrow().mutation_cursor();
        let capacity = {
            let dom = state.dom.borrow(); let mut node = Some(range.start_container); let mut count = 0usize;
            while let Some(current) = node {
                count = count.checked_add(1).ok_or(V8BindingError::Allocation("cross-link ancestor overflow"))?;
                if count > 16384 { return Err(V8BindingError::Allocation("cross-link ancestor capacity")); }
                node = dom.parent(current)?;
            }
            count
        };
        let allocation = context.retain_metadata(capacity.checked_mul(std::mem::size_of::<NodeId>())
            .ok_or(V8BindingError::Allocation("cross-link metadata overflow"))?).await?;
        synchronize_selection_range(state)?;
        if state.selection_range.get() != association || state.selection_revision.get() != revision
            || state.dom.borrow().mutation_cursor() != mutation { return Ok(()); }
        let dom = state.dom.borrow();
        let host = editing_host(state)?;
        let mut node = range.start_container;
        let mut chain = Vec::new();
        chain.try_reserve_exact(capacity).map_err(|_|V8BindingError::Allocation("cross-link chain"))?;
        if chain.capacity() != capacity { return Err(V8BindingError::Allocation("cross-link capacity overshoot")); }
        let mut found = None;
        loop {
            if Some(node) == host { break; }
            if dom.is_element(node, "http://www.w3.org/1999/xhtml", "a")
                && dom.arena().get(node)?.attributes.get_null("href").is_some() {
                let (_, after) = relative_boundary(&dom, node, true)?;
                let parent = dom.parent(node)?.ok_or(V8BindingError::Allocation("crossed link parent"))?;
                if compare_boundary_points(&dom, (range.end_container, range.end_offset), (parent, after))? > 0 {
                    found = Some((node, parent, host, chain, association, revision, allocation));
                }
                break;
            }
            let Some(parent) = dom.parent(node)? else { break; };
            if node != range.start_container {
                if is_editing_block(&dom, node) || dom.children(node)?.len() != 1 { break; }
                chain.push(node);
            }
            node = parent;
        }
        found
    } else { None };
    if let Some((link, _, _, chain, _, _, _)) = &crossed_link {
        context.lease(*link)?;
        for node in chain { context.lease(*node)?; }
    }
    if has_selection {
        editing_range::contents(context, &range, RangeContentsMode::Delete).await?;
        if let Some((start_block, end_block)) = affected_blocks
            && state.dom.borrow().arena().get(start_block).is_ok()
            && state.dom.borrow().arena().get(end_block).is_ok()
            && state.dom.borrow().parent(start_block)? == state.dom.borrow().parent(end_block)?
        {
            let end_children = light_children(&state.dom.borrow(), end_block)?;
            if node_is_placeholder_empty(&state.dom.borrow(), end_block)? {
                let _parent = state
                    .dom
                    .borrow()
                    .parent(end_block)?
                    .ok_or(V8BindingError::Allocation("editing block parent"))?;
                context.remove(end_block).await?;
            } else {
                for child in end_children {
                    context.append(start_block, child, None).await?;
                }
                let _parent = state
                    .dom
                    .borrow()
                    .parent(end_block)?
                    .ok_or(V8BindingError::Allocation("editing block parent"))?;
                context.remove(end_block).await?;
            }
            if light_children(&state.dom.borrow(), start_block)?.is_empty() {
                let br = state.dom.borrow_mut().create_element("br");
                context.append(start_block, br, None).await?;
            }
        }
    }
    let mut point = BoundaryPoint {
        container: if preserve_nested_caret {
            range.end_container
        } else {
            range.start_container
        },
        offset: if preserve_nested_caret {
            0
        } else {
            range.start_offset
        },
    };
    if let Some((link, parent, host, chain, association, revision, _allocation)) = crossed_link {
        let eligible = {
        let dom = state.dom.borrow();
        if state.selection_range.get() == association && state.selection_revision.get() == revision
            && dom.is_connected(link)? && dom.parent(link)? == Some(parent)
            && editing_host_for_node(&dom, Some(link), state.design_mode_enabled(), state.document_root())? == host
            && editing_host(state)? == host {
            let live = state.selection_range.get().and_then(|id| state.ranges.borrow().get(&id).cloned());
            if live.as_ref().is_some_and(|live| live.start_container == range.start_container
                && live.start_offset == range.start_offset) {
                true
            } else { false }
        } else { false }
        };
        if eligible {
            // The helper tracks the actual still-live selected range, not a
            // synthesized caret. Native moves update both range endpoints;
            // replacement collapse remains this function's final operation.
            if !chain.is_empty() {
                if let Some(host) = host {
                    if !lift_link_inline_chain(context, link, &chain, range.start_container, host, true).await? { return Ok(()); }
                }
            }
            let dom = state.dom.borrow();
            if state.selection_range.get() == association && state.selection_revision.get() == revision
                && dom.is_connected(link)? && editing_host(state)? == host {
                let (container, offset) = relative_boundary(&dom, link, true)?;
                point = BoundaryPoint { container, offset };
            }
        }
    }
    if !replacement.is_empty() {
        // A container boundary beside existing text is the same insertion
        // position. Reuse that source, rather than creating adjacent Text nodes.
        let adjacent = {
            let dom = state.dom.borrow();
            if matches!(
                dom.arena().get(point.container)?.kind,
                NodeKind::Element { .. }
            ) {
                let children = light_children(&dom, point.container)?;
                let index = usize::try_from(point.offset)
                    .map_err(|_| V8BindingError::Allocation("editing boundary index"))?;
                let host = editing_host_for_node(
                    &dom,
                    Some(point.container),
                    state.design_mode_enabled(),
                    state.document_root(),
                )?;
                if let Some(host) = host {
                    let layout = state.layout.borrow();
                    let block = nearest_rendered_editing_block_in_layout(
                        &dom,
                        &layout,
                        point.container,
                        host,
                    )?;
                    let eligible = |node: NodeId| -> Result<bool, V8BindingError> {
                        Ok(matches!(dom.arena().get(node)?.kind, NodeKind::Text { .. })
                            && editing_host_for_node(
                                &dom,
                                Some(node),
                                state.design_mode_enabled(),
                                state.document_root(),
                            )? == Some(host)
                            && nearest_rendered_editing_block_in_layout(&dom, &layout, node, host)?
                                == block)
                    };
                    // The native command prefers the preceding Text. Read its
                    // UTF-16 length only after confirming eligibility.
                    if let Some(node) = index
                        .checked_sub(1)
                        .and_then(|index| children.get(index))
                        .copied()
                        .filter(|node| {
                            matches!(
                                dom.arena().get(*node).map(|record| &record.kind),
                                Ok(NodeKind::Text { .. })
                            )
                        })
                    {
                        if eligible(node)? {
                            Some(BoundaryPoint {
                                container: node,
                                offset: boundary_length(&dom, node)?,
                            })
                        } else {
                            None
                        }
                    } else if let Some(node) = children.get(index).copied() {
                        if eligible(node)? {
                            Some(BoundaryPoint {
                                container: node,
                                offset: 0,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(adjacent) = adjacent {
            context.lease(adjacent.container)?;
            point = adjacent;
        }
        let kind = state
            .dom
            .borrow()
            .arena()
            .get(point.container)?
            .kind
            .clone();
        match kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                let _ = data;
                context
                    .text(point.container, point.offset, 0, replacement)
                    .await?;
                point.offset = point
                    .offset
                    .saturating_add(oryn_dom::dom_string::len(replacement) as u32);
            }
            _ => {
                let text = state.dom.borrow_mut().create_text(replacement);
                context
                    .insert_at_boundary(point.container, point.offset, text)
                    .await?;
                let (container, offset) = relative_boundary(&state.dom.borrow(), text, true)?;
                point = BoundaryPoint { container, offset };
            }
        }
    }
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

pub(super) async fn split_editing_paragraph(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let overrides = state.editing_overrides.borrow().clone();
    let result = split_editing_paragraph_contents(context).await;
    context.overrides(overrides).await?;
    result
}

pub(super) async fn split_editing_paragraph_contents(
    context: &Context,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let Some(range_id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&range_id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        replace_selection_with_text(context, "").await?;
    }
    context.layout().await?;
    synchronize_live_ranges(state)?;
    let range = state
        .ranges
        .borrow()
        .get(&range_id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let block = nearest_rendered_editing_block(state, range.start_container, host)?;
    // These containers represent several lines or table structure. A paragraph
    // command must not clone the container itself into a sibling table cell or
    // a second preformatted/address element.
    let multiline_container = matches!(
        element_local_name(&state.dom.borrow(), block),
        Some("pre" | "listing" | "address" | "td" | "th" | "caption")
    );
    let (host_accepts_paragraphs, retain_host_paragraph_prefix) = {
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let flow_container = matches!(
            element_local_name(&dom, host),
            Some(
                "body"
                    | "div"
                    | "main"
                    | "section"
                    | "article"
                    | "aside"
                    | "nav"
                    | "header"
                    | "footer"
                    | "blockquote"
                    | "fieldset"
                    | "figure"
                    | "figcaption"
                    | "li"
                    | "dd"
            )
        );
        let block_display = layout
            .styles
            .get(&node_key(host))
            .and_then(|style| style.get("display"))
            .map_or_else(
                || is_editing_block(&dom, host) || element_local_name(&dom, host) == Some("body"),
                |display| {
                    matches!(
                        display.as_str(),
                        "block" | "flow-root" | "list-item" | "flex" | "grid"
                    )
                },
            );
        // A paragraph editing root cannot itself be split into an outer
        // sibling: it is the editing boundary. Preserve its existing prefix
        // and create the next paragraph inside that boundary instead. Generic
        // phrasing roots (for example a block-styled span) remain line-break
        // hosts; CSS block display alone does not change their content model.
        let paragraph_root = matches!(&dom.arena().get(host)?.kind,
            NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "p");
        ((flow_container || paragraph_root) && block_display, paragraph_root)
    };
    if (block == host && !host_accepts_paragraphs)
        || multiline_container
        || *state.default_paragraph_separator.borrow() == "br"
    {
        return insert_editing_line_break(context).await;
    }
    // Capture existing block boundaries before splitting inline ancestors.
    let mut block_siblings = std::collections::HashSet::new();
    if block == host {
        // The preceding scoped layout demand prepared the rendered-block snapshot.
        // Reusing that snapshot avoids an O(children) sequence of full layout
        // refreshes for every paragraph command in a block editing host.
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let children = light_children(&dom, host)?;
        for child in children {
            let display = layout
                .styles
                .get(&node_key(child))
                .and_then(|style| style.get("display"));
            let is_block = display.map_or_else(
                || is_editing_block(&dom, child),
                |display| {
                    matches!(
                        display.as_str(),
                        "block"
                            | "flow-root"
                            | "list-item"
                            | "table-cell"
                            | "table-caption"
                            | "flex"
                            | "grid"
                    )
                },
            );
            if is_block {
                block_siblings.insert(child);
            }
        }
    }
    let start_kind = state
        .dom
        .borrow()
        .arena()
        .get(range.start_container)?
        .kind
        .clone();
    let mut right = match start_kind {
        NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
            context
                .split(range.start_container, range.start_offset)
                .await?
        }
        _ => {
            let children = light_children(&state.dom.borrow(), range.start_container)?;
            if let Some(child) = children.get(range.start_offset as usize).copied() {
                child
            } else {
                let br = state.dom.borrow_mut().create_element("br");
                context.append(range.start_container, br, None).await?;
                br
            }
        }
    };

    loop {
        let parent = state
            .dom
            .borrow()
            .parent(right)?
            .ok_or(V8BindingError::Allocation("editing split parent"))?;
        if parent == block {
            break;
        }
        let (clone, siblings, index) = {
            let mut dom = state.dom.borrow_mut();
            let clone = clone_editing_element(&mut dom, parent)?;
            let siblings = light_children(&dom, parent)?;
            let index = siblings
                .iter()
                .position(|candidate| *candidate == right)
                .ok_or(V8BindingError::Allocation("editing split child"))?;
            (clone, siblings, index)
        };
        for child in siblings[index..].iter().copied() {
            context.append(clone, child, None).await?;
        }
        let grand = state
            .dom
            .borrow()
            .parent(parent)?
            .ok_or(V8BindingError::Allocation("editing split ancestor"))?;
        context.after(grand, parent, clone).await?;
        right = clone;
    }

    let siblings = light_children(&state.dom.borrow(), block)?;
    let index = siblings
        .iter()
        .position(|candidate| *candidate == right)
        .ok_or(V8BindingError::Allocation("editing paragraph boundary"))?;
    let new_block;
    if block == host {
        // Wrap the current inline run only. Other paragraphs retain their
        // identity, position and contents.
        let start = siblings[..index]
            .iter()
            .rposition(|node| block_siblings.contains(node))
            .map_or(0, |position| position + 1);
        let end = siblings[index..]
            .iter()
            .position(|node| block_siblings.contains(node))
            .map_or(siblings.len(), |position| index + position);
        let separator = state.default_paragraph_separator.borrow().clone();
        let (left_block, right_block) = {
            let mut dom = state.dom.borrow_mut();
            let needs_div = siblings[start..end].iter().any(|node| {
                traversal_nodes(&dom, *node)
                    .is_ok_and(|nodes| nodes.into_iter().any(|child| is_editing_block(&dom, child)))
            });
            let tag = if separator.as_str() == "p" && !needs_div {
                "p"
            } else {
                "div"
            };
            let left = if retain_host_paragraph_prefix { None } else { Some(dom.create_element(tag)) };
            (left, dom.create_element(tag))
        };
        new_block = right_block;
        if retain_host_paragraph_prefix {
            // The original left inline chain remains in the editing root.
            // Only the suffix becomes the next paragraph. Use the same native
            // mutation boundaries as ordinary wrapping for callbacks/undo.
            context.append(host, new_block, siblings.get(index).copied()).await?;
        } else {
            let left_block = left_block.expect("flow-container paragraph prefix");
            context
                .append(host, left_block, siblings.get(start).copied())
                .await?;
            context.after(host, left_block, new_block).await?;
            for child in siblings[start..index].iter().copied() {
                context.append(left_block, child, None).await?;
            }
        }
        for child in siblings[index..end].iter().copied() {
            context.append(new_block, child, None).await?;
        }
        if let Some(left_block) = left_block {
            ensure_split_paragraph_placeholder(context, left_block).await?;
        }
        ensure_split_paragraph_placeholder(context, new_block).await?;
        // Host wrapping uses the transaction snapshot for undo; the special
        // block-split undo assumes that both halves existed as sibling blocks.
    } else {
        new_block = clone_editing_element(&mut state.dom.borrow_mut(), block)?;
        for child in siblings[index..].iter().copied() {
            context.append(new_block, child, None).await?;
        }
        let left_placeholder = ensure_split_paragraph_placeholder(context, block).await?;
        let right_placeholder = ensure_split_paragraph_placeholder(context, new_block).await?;
        let parent = state
            .dom
            .borrow()
            .parent(block)?
            .ok_or(V8BindingError::Allocation("editing block parent"))?;
        context.after(parent, block, new_block).await?;
        context.retain_paragraph_split(block, new_block, [left_placeholder, right_placeholder])?;
    }
    let caret = traversal_nodes(&state.dom.borrow(), new_block)?
        .into_iter()
        .find(|node| {
            state.dom.borrow().arena().get(*node).is_ok_and(|record| {
                matches!(
                    record.kind,
                    NodeKind::Text { .. } | NodeKind::CDataSection { .. }
                )
            })
        })
        .unwrap_or(new_block);
    let point = BoundaryPoint {
        container: caret,
        offset: 0,
    };
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

pub(super) async fn merge_document_root_editing_boundary(
    context: &Context,
    container: NodeId,
    host: NodeId,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    if element_local_name(&state.dom.borrow(), host) != Some("html") {
        return Ok(false);
    }
    let (current, adjacent) = {
        let dom = state.dom.borrow();
        let Some(current) = document_root_editing_unit(&dom, container, host)? else {
            return Ok(false);
        };
        let mut units = Vec::new();
        collect_document_root_editing_units(&dom, host, &mut units)?;
        let Some(index) = units.iter().position(|candidate| *candidate == current) else {
            return Ok(false);
        };
        let adjacent = if forward {
            units.get(index + 1).copied()
        } else {
            index
                .checked_sub(1)
                .and_then(|index| units.get(index))
                .copied()
        };
        (current, adjacent)
    };
    let Some(adjacent) = adjacent else {
        return Ok(false);
    };
    let (left, right) = if forward {
        (current, adjacent)
    } else {
        (adjacent, current)
    };
    {
        let dom = state.dom.borrow();
        if !matches!(element_local_name(&dom, left), Some("body")) && !is_editing_block(&dom, left)
            || !matches!(element_local_name(&dom, right), Some("body"))
                && !is_editing_block(&dom, right)
        {
            return Ok(false);
        }
    }

    let destination = {
        let dom = state.dom.borrow();
        let left_in_body = has_ancestor_element_before(&dom, left, "body", host);
        let right_in_body = has_ancestor_element_before(&dom, right, "body", host);
        if right_in_body && !left_in_body {
            right
        } else {
            left
        }
    };
    let source = if destination == left { right } else { left };
    let destination_was_empty = node_is_placeholder_empty(&state.dom.borrow(), destination)?;
    let source_was_empty = node_is_placeholder_empty(&state.dom.borrow(), source)?;
    let destination_children = light_children(&state.dom.borrow(), destination)?;
    let source_children = light_children(&state.dom.borrow(), source)?;
    let caret_offset = if destination == left {
        editing_caret_end(&state.dom.borrow(), destination)?
    } else {
        source_children
            .iter()
            .filter(|child| !is_preserved_editing_boundary(&state.dom.borrow(), **child))
            .count() as u32
    };
    let moving = source_children
        .into_iter()
        .filter(|child| !is_preserved_editing_boundary(&state.dom.borrow(), *child))
        .collect::<Vec<_>>();
    if destination_was_empty {
        for child in destination_children {
            let is_break = element_local_name(&state.dom.borrow(), child) == Some("br");
            if is_break {
                context.remove(child).await?;
            }
        }
    }
    if destination == left {
        for child in moving {
            context.append(destination, child, None).await?;
        }
    } else {
        let before = light_children(&state.dom.borrow(), destination)?
            .first()
            .copied();
        for child in moving {
            context.append(destination, child, before).await?;
        }
    }
    let remove_source = {
        let dom = state.dom.borrow();
        (source_was_empty || light_children(&dom, source)?.is_empty())
            && element_local_name(&dom, source) != Some("body")
            && dom.parent(source)?.is_some()
    };
    if remove_source {
        context.remove(source).await?;
    }
    let point = if node_is_placeholder_empty(&state.dom.borrow(), destination)? {
        editing_placeholder_caret(&state.dom.borrow(), destination)?
    } else {
        BoundaryPoint {
            container: destination,
            offset: caret_offset.min(boundary_length(&state.dom.borrow(), destination)?),
        }
    };
    set_selection_boundaries(state, point, point)?;
    Ok(true)
}

pub(super) async fn remove_line_before_nested_block(
    context: &Context,
    container: NodeId,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    let block = nearest_editing_block(&state.dom.borrow(), container, host)?;
    if block == host {
        return Ok(false);
    }
    let (_parent, preceding) = {
        let dom = state.dom.borrow();
        let Some(parent) = dom.parent(block)? else {
            return Ok(false);
        };
        let children = light_children(&dom, parent)?;
        let Some(index) = children.iter().position(|candidate| *candidate == block) else {
            return Ok(false);
        };
        let preceding = index
            .checked_sub(1)
            .and_then(|index| children.get(index))
            .copied();
        (parent, preceding)
    };
    let Some(preceding) = preceding else {
        return Ok(false);
    };
    let trailing_newline = {
        let dom = state.dom.borrow();
        if editing_context_preserves_newlines(&dom, preceding, host)? {
            rightmost_text_with_trailing_newline(&dom, preceding)?
        } else {
            None
        }
    };
    if let Some((trailing_text, length)) = trailing_newline {
        context
            .text(trailing_text, length.saturating_sub(1), 1, "")
            .await?;
        if matches!(
            state.dom.borrow().arena().get(preceding)?.kind,
            NodeKind::Element { .. }
        ) && node_is_placeholder_empty(&state.dom.borrow(), preceding)?
        {
            context.remove(preceding).await?;
        }
        return Ok(true);
    }
    if element_local_name(&state.dom.borrow(), preceding) == Some("br")
        || (matches!(
            state.dom.borrow().arena().get(preceding)?.kind,
            NodeKind::Element { .. }
        ) && is_mergeable_editing_sibling(&state.dom.borrow(), preceding)
            && node_is_placeholder_empty(&state.dom.borrow(), preceding)?)
    {
        context.remove(preceding).await?;
        return Ok(true);
    }
    let preceding_can_own_editable_line = !matches!(
        element_local_name(&state.dom.borrow(), preceding),
        Some(
            "html"
                | "head"
                | "body"
                | "meta"
                | "link"
                | "base"
                | "template"
                | "title"
                | "style"
                | "script"
        )
    );
    let trailing_break = preceding_can_own_editable_line
        .then(|| traversal_nodes(&state.dom.borrow(), preceding))
        .transpose()?
        .into_iter()
        .flatten()
        .rev()
        .find(|candidate| element_local_name(&state.dom.borrow(), *candidate) == Some("br"));
    let trailing_break_owner = trailing_break
        .map(|trailing_break| {
            state
                .dom
                .borrow()
                .parent(trailing_break)
                .map(|owner| owner.map(|owner| (trailing_break, owner)))
        })
        .transpose()?
        .flatten();
    if let Some((trailing_break, _owner)) = trailing_break_owner {
        context.remove(trailing_break).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn ensure_split_paragraph_placeholder(
    context: &Context,
    block: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let state = &context.state;
    let (parent, br) = {
        let mut dom = state.dom.borrow_mut();
        let nodes = traversal_nodes(&dom, block)?;
        for node in &nodes {
            match &dom.arena().get(*node)?.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } if !data.is_empty() => {
                    return Ok(None);
                }
                NodeKind::Element { name }
                    if matches!(
                        name.local.as_ref(),
                        "br" | "img"
                            | "input"
                            | "hr"
                            | "audio"
                            | "video"
                            | "canvas"
                            | "iframe"
                            | "embed"
                            | "object"
                            | "select"
                            | "textarea"
                    ) =>
                {
                    return Ok(None);
                }
                _ => {}
            }
        }
        let parent = nodes
            .iter()
            .rev()
            .copied()
            .find(|node| {
                dom.arena()
                    .get(*node)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
            })
            .unwrap_or(block);
        let br = dom.create_element("br");
        (parent, br)
    };
    context.append(parent, br, None).await?;
    Ok(Some(br))
}

pub(super) async fn remove_selection_format(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let selected = state
        .selection_range
        .get()
        .and_then(|id| state.ranges.borrow().get(&id).cloned());
    let Some(range) = selected else {
        return Ok(());
    };
    let removable = traversal_nodes(&state.dom.borrow(), host)?
        .into_iter()
        .filter(|node| {
            let dom = state.dom.borrow();
            let Ok(before) = relative_boundary(&dom, *node, false) else {
                return false;
            };
            let Ok(after) = relative_boundary(&dom, *node, true) else {
                return false;
            };
            let intersects =
                compare_boundary_points(&dom, (range.end_container, range.end_offset), before)
                    .is_ok_and(|ordering| ordering > 0)
                    && compare_boundary_points(
                        &dom,
                        (range.start_container, range.start_offset),
                        after,
                    )
                    .is_ok_and(|ordering| ordering < 0);
            intersects
                && matches!(
                    element_local_name(&dom, *node),
                    Some(
                        "a" | "b"
                            | "font"
                            | "i"
                            | "s"
                            | "span"
                            | "strike"
                            | "strong"
                            | "sub"
                            | "sup"
                            | "u"
                    )
                )
        })
        .collect::<Vec<_>>();
    for node in removable.into_iter().rev() {
        if state.dom.borrow().arena().get(node).is_ok() {
            let (parent, children) = {
                let dom = state.dom.borrow();
                (
                    dom.parent(node)?.ok_or_else(hierarchy_request_error)?,
                    light_children(&dom, node)?,
                )
            };
            let _parent = context.lease(parent)?;
            let _node = context.lease(node)?;
            for child in &children {
                context.lease(*child)?;
            }
            for child in children {
                context.append(parent, child, Some(node)).await?;
            }
            context.remove(node).await?;
        }
    }
    synchronize_live_ranges(state)
}

pub(super) async fn clear_contained_alignment(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    if state.selection_anchor.get().is_none() || editing_host(state)?.is_none() { return Ok(()); }
    context.layout().await?;
    let Some(range) = editing_alignment_range(state)? else {
        return Ok(());
    };
    let nodes = contained_alignment_nodes(state, &range)?;
    for node in nodes {
        let needs_cleanup = {
            let dom = state.dom.borrow();
            matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. })
                && (dom.attribute_in_null_namespace(node, "align")?.is_some()
                    || element_local_name(&dom, node) == Some("center")
                    || dom
                        .attribute_in_null_namespace(node, "style")?
                        .is_some_and(|style| {
                            style.split(';').any(|declaration| {
                                declaration.split_once(':').is_some_and(|(name, _)| {
                                    name.trim().eq_ignore_ascii_case("text-align")
                                })
                            })
                        }))
        };
        if !needs_cleanup {
            continue;
        }
        context.remove_attribute(node, "align").await?;
        context.style(node, "text-align", "").await?;
        let empty_style = state
            .dom
            .borrow()
            .attribute_in_null_namespace(node, "style")?
            == Some("");
        if empty_style {
            context.remove_attribute(node, "style").await?;
        }
        let (name, attributes, parent) = {
            let dom = state.dom.borrow();
            (
                element_local_name(&dom, node).unwrap_or("").to_owned(),
                dom.arena().get(node)?.attributes.clone(),
                dom.parent(node)?,
            )
        };
        let Some(parent) = parent else {
            continue;
        };
        if matches!(name.as_str(), "div" | "span" | "center") && attributes.is_empty() {
            let children = light_children(&state.dom.borrow(), node)?;
            for child in children {
                let (_, index) = relative_boundary(&state.dom.borrow(), node, false)?;
                context.move_preserving_ranges(child, parent, index).await?;
            }
            context.remove(node).await?;
            synchronize_live_ranges(state)?;
        } else if name == "center" {
            let replacement = state.dom.borrow_mut().create_element("div");
            for attribute in attributes.records() {
                context
                    .attribute(
                        replacement,
                        attribute.namespace.clone(),
                        attribute.name.clone(),
                        attribute.value.clone(),
                    )
                    .await?;
            }
            context.append(parent, replacement, Some(node)).await?;
            let children = light_children(&state.dom.borrow(), node)?;
            for child in children {
                let index = boundary_length(&state.dom.borrow(), replacement)?;
                context
                    .move_preserving_ranges(child, replacement, index)
                    .await?;
            }
            context.remove(node).await?;
            synchronize_live_ranges(state)?;
        }
    }
    Ok(())
}

pub(super) async fn separate_alignment_inline_runs(
    context: &Context,
    wrapper: NodeId,
    incoming: &[NodeId],
    prepend: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    context.layout().await?;
    let needs_break = {
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let children = light_children(&dom, wrapper)?;
        let visible = |nodes: &[NodeId]| -> Result<Vec<NodeId>, V8BindingError> {
            let mut result = Vec::new();
            for node in nodes {
                if alignment_node_is_visible(&dom, &layout, *node)? {
                    result.push(*node);
                }
            }
            Ok(result)
        };
        let existing = visible(&children)?;
        let incoming_visible = visible(incoming)?;
        let (left, right, last) = if prepend {
            (incoming_visible.last(), existing.first(), incoming.last())
        } else {
            (existing.last(), incoming_visible.first(), children.last())
        };
        alignment_node_is_block(&dom, &layout, wrapper)
            && left.zip(right).is_some_and(|(left, right)| {
                !alignment_node_is_block(&dom, &layout, *left)
                    && !alignment_node_is_block(&dom, &layout, *right)
            })
            && !last.is_some_and(|node| element_local_name(&dom, *node) == Some("br"))
    };
    if needs_break {
        let reference = if prepend {
            light_children(&state.dom.borrow(), wrapper)?
                .first()
                .copied()
        } else {
            None
        };
        let br = state.dom.borrow_mut().create_element("br");
        context.append(wrapper, br, reference).await?;
    }
    Ok(())
}

pub(super) async fn wrap_alignment_contents(
    context: &Context,
    alignment: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    if state.selection_anchor.get().is_none() || editing_host(state)?.is_none() { return Ok(()); }
    context.layout().await?;
    let Some(range) = editing_alignment_range(state)? else {
        return Ok(());
    };
    let mut selected = Vec::new();
    for node in contained_alignment_nodes(state, &range)? {
        let covered = {
            let dom = state.dom.borrow();
            let mut parent = dom.parent(node)?;
            let mut covered = false;
            while let Some(ancestor) = parent {
                if selected.contains(&ancestor) {
                    covered = true;
                    break;
                }
                parent = dom.parent(ancestor)?;
            }
            covered || !alignment_allows_div_child(&dom, node)?
        };
        if !covered && editing_alignment_value(state, node)? != alignment {
            selected.push(node);
        }
    }
    let mut index = 0;
    while index < selected.len() {
        let first = selected[index];
        let (parent, position) = relative_boundary(&state.dom.borrow(), first, false)?;
        let mut end = index + 1;
        {
            let dom = state.dom.borrow();
            let siblings = light_children(&dom, parent)?;
            while end < selected.len()
                && siblings.get(position as usize + end - index) == Some(&selected[end])
            {
                end += 1;
            }
        }
        let mut has_visible_content = false;
        {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            for node in &selected[index..end] {
                if alignment_node_is_visible(&dom, &layout, *node)?
                    || element_local_name(&dom, *node) == Some("br")
                {
                    has_visible_content = true;
                    break;
                }
            }
        }
        if !has_visible_content {
            index = end;
            continue;
        }
        let (run, position) = {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            let siblings = light_children(&dom, parent)?;
            let mut start = position as usize;
            let mut limit = start + end - index;
            let last = siblings[limit - 1];
            if !alignment_node_is_block(&dom, &layout, last)
                && element_local_name(&dom, last) != Some("br")
                && siblings
                    .get(limit)
                    .is_some_and(|node| element_local_name(&dom, *node) == Some("br"))
            {
                limit += 1;
            }
            while start > 0 && !alignment_node_is_visible(&dom, &layout, siblings[start - 1])? {
                start -= 1;
            }
            while limit < siblings.len()
                && !alignment_node_is_visible(&dom, &layout, siblings[limit])?
            {
                limit += 1;
            }
            (siblings[start..limit].to_vec(), start as u32)
        };
        while end < selected.len() && run.contains(&selected[end]) {
            end += 1;
        }
        let first = run[0];
        let (previous, next) = {
            let dom = state.dom.borrow();
            let siblings = light_children(&dom, parent)?;
            (
                position
                    .checked_sub(1)
                    .and_then(|i| siblings.get(i as usize))
                    .copied()
                    .filter(|node| reusable_alignment_wrapper(&dom, *node, alignment)),
                siblings
                    .get(position as usize + run.len())
                    .copied()
                    .filter(|node| reusable_alignment_wrapper(&dom, *node, alignment)),
            )
        };
        let wrapper = if let Some(previous) = previous {
            previous
        } else if let Some(next) = next {
            next
        } else {
            let wrapper = state.dom.borrow_mut().create_element("div");
            context.style(wrapper, "text-align", alignment).await?;
            context
                .insert_alignment_wrapper(parent, wrapper, first, position)
                .await?;
            wrapper
        };
        let prepend = previous.is_none() && next.is_some();
        separate_alignment_inline_runs(context, wrapper, &run, prepend).await?;
        if prepend {
            for node in run.iter().rev() {
                context.move_preserving_ranges(*node, wrapper, 0).await?;
            }
        } else {
            for node in &run {
                let offset = boundary_length(&state.dom.borrow(), wrapper)?;
                context
                    .move_preserving_ranges(*node, wrapper, offset)
                    .await?;
            }
        }
        if let Some(next) = next.filter(|next| *next != wrapper) {
            let children = light_children(&state.dom.borrow(), next)?;
            separate_alignment_inline_runs(context, wrapper, &children, false).await?;
            for child in children {
                let offset = boundary_length(&state.dom.borrow(), wrapper)?;
                context
                    .move_preserving_ranges(child, wrapper, offset)
                    .await?;
            }
            context.remove(next).await?;
            synchronize_live_ranges(state)?;
        }
        index = end;
    }
    synchronize_live_ranges(state)
}

pub(super) async fn remove_editing_placeholder_before_typing(
    context: &Context,
    text: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    if text.is_empty() {
        return Ok(());
    }
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    context.layout().await?;
    let Some(range) = state
        .selection_range
        .get()
        .and_then(|id| state.ranges.borrow().get(&id).cloned())
    else {
        return Ok(());
    };
    let whitespace = editing_white_space(state, range.start_container)?;
    if !editing_text_has_visible_content(text, &whitespace) {
        // A typed space on an empty line becomes a visible NBSP. It replaces
        // the placeholder, while a break following existing content remains.
        let point = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
        let run = editing_space_run(state, point, host)?;
        if !text.contains(' ') || !run.starts_line || !run.ends_line {
            return Ok(());
        }
    }
    let block = nearest_rendered_editing_block(state, range.start_container, host)?;
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    {
        let dom = state.dom.borrow();
        if matches!(
            dom.arena().get(point.container)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            if point.offset != boundary_length(&dom, point.container)? {
                return Ok(());
            }
            let (container, offset) = relative_boundary(&dom, point.container, true)?;
            point = BoundaryPoint { container, offset };
        }
    }
    loop {
        let candidate = light_children(&state.dom.borrow(), point.container)?
            .get(point.offset as usize)
            .copied();
        if let Some(candidate) = candidate {
            if element_local_name(&state.dom.borrow(), candidate) != Some("br") {
                return Ok(());
            }
            let after = BoundaryPoint {
                container: point.container,
                offset: point.offset + 1,
            };
            if !editing_line_has_following_content(state, after, host, &whitespace)? {
                context.remove(candidate).await?;
            }
            return Ok(());
        }
        if point.container == block {
            return Ok(());
        }
        let (container, offset) = relative_boundary(&state.dom.borrow(), point.container, true)?;
        point = BoundaryPoint { container, offset };
    }
}
