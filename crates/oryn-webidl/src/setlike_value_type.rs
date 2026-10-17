fn idl_setlike_value_name(value:&TypeDescriptor)->String {
    match value {
        TypeDescriptor::Named(name)=>name.clone(),
        TypeDescriptor::Nullable(inner)=>format!("{}?",idl_setlike_value_name(inner)),
        TypeDescriptor::Sequence(inner)=>format!("sequence<{}>",idl_setlike_value_name(inner)),
        TypeDescriptor::Promise(inner)=>format!("Promise<{}>",idl_setlike_value_name(inner)),
        TypeDescriptor::FrozenArray(inner)=>format!("FrozenArray<{}>",idl_setlike_value_name(inner)),
        TypeDescriptor::ObservableArray(inner)=>format!("ObservableArray<{}>",idl_setlike_value_name(inner)),
        TypeDescriptor::Record(key,value)=>format!("record<{}, {}>",idl_setlike_value_name(key),idl_setlike_value_name(value)),
        TypeDescriptor::Union(values)=>format!("({})",values.iter().map(idl_setlike_value_name).collect::<Vec<_>>().join(" or ")),
        TypeDescriptor::Attributed(attributes,inner)=>{
            let attributes=attributes.iter().map(|attribute|match &attribute.value{
                Some(value)=>format!("{}={value}",attribute.name),None=>attribute.name.clone(),
            }).collect::<Vec<_>>().join(", ");
            format!("[{attributes}] {}",idl_setlike_value_name(inner))
        },
    }
}
