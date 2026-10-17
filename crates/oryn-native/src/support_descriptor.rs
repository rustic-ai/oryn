//! Pinned support closure; native fixture-contract loader alone installs it.
//! Fixed metadata: no runtime caller assertion list or mutable URL authority.
use sha2::{Digest, Sha256};

pub(crate) const PARENT: &str = "/IndexedDB/idb-partitioned-coverage.sub.html";
pub(crate) const CHILD: &str = "/IndexedDB/resources/idb-partitioned-coverage-iframe.html";
pub(crate) const REVISION: &str = "719d5e38fdd0903a18ed9007aba816c98cc491e0";
pub(crate) const IMPORTS: [(&str, &str); 6] = [
    ("/resources/testharness.js", "d2399236c2a09c429804ff2299ad6629e17e2b53f17a74dd341e936adb11ae3e"),
    ("/IndexedDB/resources/support.js", "3bbc4fa7b1a4e2354a05322b4fd4e3ac1afad6ad065af746d9fc377ee28ef1a7"),
    ("/IndexedDB/resources/nested-cloning-common.js", "579c2829a8ed271c6baa0e66c80660bb5b39cb01914637b4dfdc0170700d0f07"),
    ("/IndexedDB/resources/support-get-all.js", "377019848bf1ef23e74a90470a5ddfcbf55e3ff82c35be0ebd3e9c7e4e443444"),
    ("/IndexedDB/resources/support-promises.js", "b17a508df2752a4240e898013fd47313e4742b6f195d3e1342fbb7134d5948fb"),
    ("/IndexedDB/idbindex_getAll.any.js", "3b6bcf5a4d021a420d3d8a30ef5803cee398e3312bde589e25a43b9c7647906d"),
];

#[derive(Clone, Copy, Debug)]
pub struct PinnedSupportDescriptor {
    candidate: [u8; 32],
    descriptor: [u8; 32],
    parent_url: [u8; 32],
    child_url: [u8; 32],
    child_body: [u8; 32],
    imports: [[u8; 32]; 6],
    import_urls: [[u8; 32]; 6],
}

impl PinnedSupportDescriptor {
    /// Inputs have already passed descriptor-anchored regular-file reads and
    /// candidate binding. Recheck native constants before creating capability.
    pub fn from_fixture(candidate: [u8;32], descriptor: [u8;32],
        parent_url: &url::Url, child_url: &url::Url,
        revision: &str, parent_source: &[u8], child_source: &[u8],
        imported_sources: [&[u8]; 6]) -> Option<Self> {
        if revision != REVISION || candidate == [0;32]
            || hex(&descriptor).as_slice() != b"9b102136523d37559a98e9a101b6c7f4f379f43f01160f840a8248e51fd8a8d7"
            || parent_url.path() != PARENT || child_url.path() != CHILD
            || !parent_url.query().is_none() || !child_url.query().is_none()
            || parent_url.fragment().is_some() || child_url.fragment().is_some()
            || !matches!(parent_url.scheme(), "http" | "https")
            || !matches!(child_url.scheme(), "http" | "https") { return None; }
        let parent: [u8;32] = Sha256::digest(parent_source).into();
        let child: [u8;32] = Sha256::digest(child_source).into();
        if hex(&parent).as_slice() != b"9767126059bb9dea9a4e59b9a4f01a2a2f439d966d8c57a1e9f97104576afff0"
            || hex(&child).as_slice() != b"ba2ed49fd6b15b72e6ac2006bb61ba95fad7132d54668557c3188892891c0bc6" { return None; }
        let mut imports = [[0u8;32];6];
        let mut import_urls = [[0u8;32];6];
        for ((slot, source), (_, expected)) in imports.iter_mut().zip(imported_sources).zip(IMPORTS) {
            *slot = Sha256::digest(source).into();
            if hex(slot).as_slice() != expected.as_bytes() { return None; }
        }
        // Issuer preparation owns/admitted URL storage; runtime publication
        // compares fixed hashes and does not clone URL metadata.
        for (index, (path, _)) in IMPORTS.iter().enumerate() {
            let resource = child_url.join(path).ok()?;
            import_urls[index] = Sha256::digest(resource.as_str().as_bytes()).into();
        }
        Some(Self { candidate, descriptor, parent_url: Sha256::digest(parent_url.as_str()).into(),
            child_url: Sha256::digest(child_url.as_str()).into(), child_body: child, imports, import_urls })
    }
    pub(crate) fn response(&self, parent_url: &str, child_url: &str)
        -> Option<([u8;32], [u8;32], [u8;32])> {
        let parent: [u8;32] = Sha256::digest(parent_url.as_bytes()).into();
        let child: [u8;32] = Sha256::digest(child_url.as_bytes()).into();
        (parent == self.parent_url && child == self.child_url)
            .then_some((self.descriptor, self.candidate, self.child_body))
    }
    pub(crate) fn import(&self, index: usize, resource: &url::Url, source: &[u8]) -> bool {
        // This fixture's imports are relative/root-relative to its own child
        // origin. Equal bytes from a foreign-origin URL are not that resource.
        let actual_url: [u8;32] = Sha256::digest(resource.as_str().as_bytes()).into();
        self.import_urls.get(index) == Some(&actual_url)
            && resource.query().is_none() && resource.fragment().is_none()
            && IMPORTS.get(index).is_some_and(|(expected, _)| *expected == resource.path())
            && self.imports.get(index).is_some_and(|expected| {
                let actual: [u8;32] = Sha256::digest(source).into(); actual == *expected
            })
    }
}

fn hex(bytes: &[u8;32]) -> [u8;64] {
    const DIGITS: &[u8;16] = b"0123456789abcdef";
    let mut output = [0u8;64];
    for (index, byte) in bytes.iter().enumerate() {
        output[index*2] = DIGITS[(byte >> 4) as usize];
        output[index*2+1] = DIGITS[(byte & 15) as usize];
    }
    output
}
