#[test]
fn setlike_preserves_kind_readonly_and_attributes() {
    let parsed=parse_interface("[Exposed=Window] interface Example { [SecureContext] readonly setlike<DOMString>; };").unwrap();
    assert!(matches!(&parsed.members[0],Member::Setlike{readonly:true,value:TypeDescriptor::Named(name),extended_attributes}
        if name=="DOMString" && has_extended_attribute(extended_attributes,"SecureContext")));
    let generated=emit_binding_registry(&[Definition::Interface(parsed)]).unwrap();
    assert!(generated.contains("GeneratedMemberKind::Setlike"));
    assert!(!generated.contains("name: \"@@iterator\""));
}
#[test]
fn setlike_rejects_multiple_value_types() {
    assert!(parse_interface("interface Example { setlike<DOMString, long>; };").is_err());
}

#[test]
fn invalid_setlike_collection_declarations_are_rejected(){
    for declaration in [
        "setlike<>;",
        "static setlike<DOMString>;",
        "setlike<DOMString>; setlike<long>;",
        "iterable<DOMString>; setlike<DOMString>;",
        "setlike<DOMString>; iterable<DOMString>;",
    ] {
        assert!(parse_interface(&format!("interface Example {{ {declaration} }};")).is_err(),"{declaration}");
    }
}
