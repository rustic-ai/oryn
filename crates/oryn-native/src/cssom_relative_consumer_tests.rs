#[cfg(test)]
mod cssom_relative_consumer_tests {
    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn relative_cssom_resolves_each_inset_without_using_visual_precedence() {
        let context = crate::Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://example.test/insets", "<!doctype html><div id=cb style='width:200px;height:100px;direction:rtl'><div id=t style='position:relative;width:20px;height:20px;top:calc(10% - 1px);bottom:calc(50% - 3px);left:calc(25% - 2px);right:calc(75% - 4px)'></div></div>").await.unwrap();
        assert_eq!(page.evaluate("const t=document.getElementById('t');let s=getComputedStyle(t);[s.top,s.bottom,s.left,s.right].join('|')").await.unwrap(), "9px|47px|48px|146px");
        assert_eq!(page.evaluate("t.style.top='auto';t.style.left='auto';s=getComputedStyle(t);[s.top,s.bottom,s.left,s.right].join('|')").await.unwrap(), "-47px|47px|-146px|146px");
        assert_eq!(page.evaluate("t.style.position='static';s=getComputedStyle(t);[s.top,s.bottom,s.left,s.right].join('|')").await.unwrap(), "auto|calc(50% - 3px)|auto|calc(75% - 4px)");
        assert_eq!(page.evaluate("cb.style.writingMode='vertical-rl';t.style.position='relative';t.style.top='10%';t.style.bottom='50%';t.style.left='25%';t.style.right='75%';s=getComputedStyle(t);[s.top,s.bottom,s.left,s.right].join('|')").await.unwrap(),"10px|50px|50px|150px");
        assert_eq!(page.evaluate("cb.style.writingMode='horizontal-tb';const inline=document.createElement('span');inline.textContent='text';inline.style.cssText='position:relative;top:10%;right:4px';cb.append(inline);const si=getComputedStyle(inline);[si.top,si.bottom,si.left,si.right].join('|')").await.unwrap(),"10px|-10px|-4px|4px");
    }
}
