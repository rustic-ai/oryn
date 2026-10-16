#[cfg(test)]
mod native_declaration_conversion_control {
    use super::*;

    #[test]
    fn native_declaration_conversion_refreshes_receipt_and_retains_original_rule() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><style id=s>p{width:17px}</style><p id=p></p>",
            &BTreeMap::new(), None,
            Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            const sheet=s.sheet, rule=sheet.cssRules[0], style=rule.style;
            style.width={toString(){sheet.insertRule('q{}',1);return '19px'}};
            style.setProperty({toString(){sheet.deleteRule(1);return 'height'}},
                {toString(){sheet.insertRule('r{}',1);return '23px'}},
                {toString(){sheet.deleteRule(1);return ''}});
            const old=style.removeProperty({toString(){sheet.insertRule('t{}',1);return 'height'}});
            return [sheet.cssRules[0]===rule,rule.style===style,style.width,old,
                style.height,sheet.cssRules.length].join('|');
        })()"#).unwrap(), "true|true|19px|23px||2");
    }

    #[test]
    fn native_declaration_conversion_detaches_owner_without_retargeting_rule() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<!doctype html><style id=s>p{width:17px}</style><p></p>",
            &BTreeMap::new(), None,
            Some(Url::parse("https://example.test/").unwrap()),
        ).unwrap();
        assert_eq!(page.evaluate(r#"(() => {
            const style=s.sheet.cssRules[0].style;
            const owner=s;
            style.width={toString(){
                owner.remove();
                const replacement=document.createElement('style');
                replacement.id='replacement';replacement.textContent='p{width:29px}';
                document.head.append(replacement);
                return '99px';
            }};
            const replacement=document.getElementById('replacement');
            return [style.width,replacement.sheet.cssRules[0].style.width,
                owner.isConnected].join('|');
        })()"#).unwrap(), "99px|29px|false");
    }
}
