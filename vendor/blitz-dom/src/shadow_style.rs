//! Native shadow-tree style data for the disposable Oryn style mirror.
//! Opaque selector tokens are identities only: they are never dereferenced.
use std::{collections::HashMap, ptr::NonNull, sync::{Mutex, LazyLock, atomic::{AtomicUsize, Ordering}}};
use blitz_traits::node_id::NodeId;
use selectors::OpaqueElement;
use style::{servo_arc::Arc, stylist::CascadeData, stylesheets::{DocumentStyleSheet, scope_rule::ImplicitScopeRoot}};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
static SCOPES: LazyLock<Mutex<HashMap<OpaqueElement, Vec<Option<ImplicitScopeRoot>>>>> = LazyLock::new(||Mutex::new(HashMap::new()));
pub(crate) fn next_identity() -> usize {
    NEXT_ID.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|id|id.checked_add(1)).expect("selector identity exhausted")
}
pub(crate) fn opaque(identity:usize)->OpaqueElement {
    OpaqueElement::from_non_null_ptr(NonNull::new(identity as *mut ()).expect("nonzero native selector identity"))
}
pub(crate) fn implicit_scope(host:OpaqueElement,index:usize)->Option<ImplicitScopeRoot>{
    SCOPES.lock().unwrap_or_else(|error|error.into_inner()).get(&host)?.get(index).copied().flatten()
}
pub struct ShadowStyle {
    pub cascade: Option<Arc<CascadeData>>,
    host: OpaqueElement,
}
impl Drop for ShadowStyle {
    fn drop(&mut self){SCOPES.lock().unwrap_or_else(|error|error.into_inner()).remove(&self.host);}
}
impl crate::BaseDocument {
    /// Sheets already carry native base URLs/media and tree-local implicit scopes.
    /// Rebuild through Stylo's actual author cascade, preserving host/slotted levels.
    pub fn install_shadow_styles(&mut self,root:NodeId,sheets:Vec<DocumentStyleSheet>)->Result<(),String>{
        let host=self.get_node(root).and_then(|node|node.shadow_host).ok_or("missing native shadow host")?;
        let host=opaque(self.get_node(host).ok_or("missing native host node")?.selector_identity);
        let scopes=sheets.iter().map(|sheet|sheet.1).collect();
        let lock=self.guard().clone();let guard=lock.read();
        let mut set=style::stylesheet_set::AuthorStylesheetSet::new();
        for sheet in sheets {set.append_stylesheet(None,&Default::default(),sheet,&guard);}
        let (flusher,_)=set.flush();
        let cascade=self.stylist.rebuild_author_data(&CascadeData::new(),flusher.sheets,&guard,&mut Default::default())
            .map_err(|_|"shadow cascade allocation failed".to_owned())?;
        // Remove the old registration before replacing it; Drop must not erase
        // the newly installed registration for the same host.
        self.get_node_mut(root).ok_or("missing shadow root")?.shadow_style=None;
        SCOPES.lock().unwrap_or_else(|error|error.into_inner()).insert(host,scopes);
        self.get_node_mut(root).ok_or("missing shadow root")?.shadow_style=Some(ShadowStyle{cascade,host});
        Ok(())
    }
}
