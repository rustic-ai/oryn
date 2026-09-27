//! Owned UTF-8 results from the bounded libxslt boundary. External resources must
//! already have passed browser policy; the native loader only sees this exact map.
use std::ffi::{CString, c_char, c_int};
#[repr(C)]
struct Parameter {
    namespace_uri: *const c_char,
    local: *const c_char,
    value: *const c_char,
}
#[repr(C)]
struct Resource {
    url: *const c_char,
    base: *const c_char,
    bytes: *const u8,
    length: usize,
}
#[repr(C)]
struct Output {
    bytes: *mut u8,
    length: usize,
    kind: c_int,
}
unsafe extern "C" {
    fn oryn_xslt_validate(
        style: *const u8,
        length: usize,
        url: *const c_char,
        resources: *const Resource,
        resource_count: usize,
    ) -> c_int;
    fn oryn_xslt_transform(
        style: *const u8,
        style_length: usize,
        source: *const u8,
        source_length: usize,
        parameters: *const Parameter,
        count: usize,
        url: *const c_char,
        resources: *const Resource,
        resource_count: usize,
        output: *mut Output,
    ) -> c_int;
    fn oryn_xslt_free(bytes: *mut u8);
}
/// Stylesheet dependencies are immutable snapshots scoped to one operation.
#[derive(Default)]
pub struct Resources(pub Vec<(String, String, String)>);
impl Resources {
    fn names(&self) -> Option<Vec<(CString, CString)>> {
        self.0
            .iter()
            .map(|(url, base, _)| {
                Some((
                    CString::new(url.as_str()).ok()?,
                    CString::new(base.as_str()).ok()?,
                ))
            })
            .collect()
    }
    fn raw(&self, names: &[(CString, CString)]) -> Vec<Resource> {
        self.0
            .iter()
            .zip(names)
            .map(|((_, _, data), (name, base))| Resource {
                url: name.as_ptr(),
                base: base.as_ptr(),
                bytes: data.as_ptr(),
                length: data.len(),
            })
            .collect()
    }
}
fn validate(style: &str, url: &str, resources: &Resources) -> bool {
    let (Ok(url), Some(names)) = (CString::new(url), resources.names()) else {
        return false;
    };
    let raw = resources.raw(&names);
    unsafe {
        oryn_xslt_validate(
            style.as_ptr(),
            style.len(),
            url.as_ptr(),
            raw.as_ptr(),
            raw.len(),
        ) != 0
    }
}
fn transform(
    style: &str,
    source: &str,
    values: &[(String, String, String)],
    url: &str,
    resources: &Resources,
) -> Option<(String, i32)> {
    let url = CString::new(url).ok()?;
    let names = resources.names()?;
    let raw = resources.raw(&names);
    let strings = values
        .iter()
        .map(|(ns, local, value)| {
            Some((
                CString::new(ns.as_str()).ok()?,
                CString::new(local.as_str()).ok()?,
                CString::new(value.as_str()).ok()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let parameters = strings
        .iter()
        .map(|(ns, local, value)| Parameter {
            namespace_uri: ns.as_ptr(),
            local: local.as_ptr(),
            value: value.as_ptr(),
        })
        .collect::<Vec<_>>();
    let mut output = Output {
        bytes: std::ptr::null_mut(),
        length: 0,
        kind: 0,
    };
    // C borrows all parameters/maps only until return; the output is malloc-owned.
    if unsafe {
        oryn_xslt_transform(
            style.as_ptr(),
            style.len(),
            source.as_ptr(),
            source.len(),
            parameters.as_ptr(),
            parameters.len(),
            url.as_ptr(),
            raw.as_ptr(),
            raw.len(),
            &mut output,
        )
    } == 0
    {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(output.bytes, output.length) };
    let text = String::from_utf8(bytes.to_vec()).ok();
    unsafe {
        oryn_xslt_free(output.bytes);
    }
    text.map(|text| (text, output.kind))
}

/// Called only in the early single-operation native helper role. Never call in
/// a page actor: libxml allocation hooks deliberately belong to that process.
pub fn run_request(
    request: &crate::platform::XsltRequest,
) -> Result<crate::platform::XsltResult, String> {
    if request.style.len() > 4 * 1024 * 1024
        || request
            .source
            .as_ref()
            .is_some_and(|value| value.len() > 16 * 1024 * 1024)
        || request.resources.len() > 64
        || request.parameters.len() > 1024
        || request.url.len() > 64 * 1024
    {
        return Err("XSLT helper input limit".into());
    }
    let mut total = 0usize;
    for (url, base, text) in &request.resources {
        if url.len() > 64 * 1024 || base.len() > 64 * 1024 || text.len() > 4 * 1024 * 1024 {
            return Err("XSLT resource limit".into());
        }
        total = total
            .checked_add(text.len())
            .filter(|total| *total <= 16 * 1024 * 1024)
            .ok_or("XSLT aggregate resource limit")?;
    }
    let resources = Resources(request.resources.clone());
    match request.source.as_deref() {
        None => Ok(crate::platform::XsltResult {
            valid: validate(&request.style, &request.url, &resources),
            output: None,
        }),
        Some(source) => {
            let output = transform(
                &request.style,
                source,
                &request.parameters,
                &request.url,
                &resources,
            );
            Ok(crate::platform::XsltResult {
                valid: output.is_some(),
                output,
            })
        }
    }
}
