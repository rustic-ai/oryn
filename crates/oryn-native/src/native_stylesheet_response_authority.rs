/// Native-only response authority. Inline roots have no network receipt, and
/// fetched descendants cannot acquire clean status from a base URL.
enum NativeStylesheetResponseAuthority {
    Fetched(crate::context_service::StylesheetResponseProof),
    Inline(NativeInlineStylesheetAuthority),
}
impl NativeStylesheetResponseAuthority {
    fn origin_clean(&self) -> bool {
        match self {
            Self::Fetched(proof) => proof.origin_clean,
            Self::Inline(_) => true,
        }
    }
    fn validate_native_owner(
        &self,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<bool, HostError> {
        match self {
            Self::Inline(authority) => authority.current(host, arena),
            Self::Fetched(_) => Ok(true), // Parent guard validation is separate.
        }
    }
}
