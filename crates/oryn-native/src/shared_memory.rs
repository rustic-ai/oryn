//! Parent-owned shared backing and exact agent-cluster membership. Closing a
//! capability prevents new maps; existing OS mappings own their physical pages.

/// Inputs for allocate.
#[cfg(unix)]
pub(crate) struct SharedAllocationInputs<'a> {
    pub(crate) owner: &'a AgentOwner,
    pub(crate) byte_length: u64,
    pub(crate) maximum_length: Option<u64>,
    pub(crate) granularity: u64,
    pub(crate) guard_length: u64,
    pub(crate) wasm: bool,
    pub(crate) memory64: bool,
}
use crate::agent_registry::AgentOwner;
use oryn_js::shared_memory::{SharedMemoryDescriptor, SharedMemoryToken};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[path = "shared_memory_handoff.rs"]
mod handoff;
#[cfg(unix)]
#[path = "shared_message_backend.rs"]
mod message_backend;
#[cfg(unix)]
#[path = "shared_wait_admission.rs"]
mod wait_admission;
#[cfg(unix)]
pub(crate) use message_backend::SharedMessageBackend;
#[cfg(unix)]
pub(crate) use wait_admission::WaitAdmissionInputs;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AgentClusterId([u8; 32]);
struct Cluster {
    origin: String,
    group: Option<crate::browsing_groups::GroupToken>,
    members: BTreeSet<AgentOwner>,
}
#[cfg(unix)]
struct Backing {
    cluster: AgentClusterId,
    descriptor: SharedMemoryDescriptor,
    file: Arc<std::fs::File>,
    holders: BTreeMap<AgentOwner, usize>,
    mapping: ParentMapping,
}
#[derive(Default)]
pub(crate) struct SharedMemoryRegistry {
    clusters: BTreeMap<AgentClusterId, Cluster>,
    members: BTreeMap<AgentOwner, AgentClusterId>,
    #[cfg(unix)]
    backings: BTreeMap<SharedMemoryToken, Backing>,
    bytes: u64,
    waits: crate::shared_wait::WaitTable,
    #[cfg(unix)]
    admissions: BTreeMap<(AgentOwner, [u8; 32]), wait_admission::Lane>,
    handoffs: BTreeMap<oryn_js::shared_memory::SharedMemoryHandoff, handoff::Handoff>,
}
fn random() -> Result<[u8; 32], String> {
    let mut token = [0; 32];
    getrandom::fill(&mut token).map_err(|error| error.to_string())?;
    if token == [0; 32] {
        return Err("shared memory entropy unavailable".into());
    }
    Ok(token)
}
impl std::fmt::Debug for SharedMemoryRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedMemoryRegistry")
            .field("clusters", &self.clusters.len())
            .field("principals", &self.members.len())
            .field("retained_bytes", &self.bytes)
            .field("handoffs", &self.handoffs.len())
            .finish()
    }
}
impl SharedMemoryRegistry {
    /// Caller derives isolation from committed response policy and ancestry.
    /// A new top-level document always receives a fresh cluster identity.
    pub(crate) fn admit_root(&mut self, owner: AgentOwner, origin: String) -> Result<(), String> {
        if self.members.contains_key(&owner) {
            return Err("agent already has cluster membership".into());
        }
        if self.clusters.len() >= 1024 {
            return Err("isolated cluster capacity".into());
        }
        let id = AgentClusterId(random()?);
        if self.clusters.contains_key(&id) {
            return Err("cluster identity collision".into());
        }
        self.clusters.insert(
            id,
            Cluster {
                origin,
                group: None,
                members: BTreeSet::from([owner.clone()]),
            },
        );
        self.members.insert(owner, id);
        Ok(())
    }
    pub(crate) fn admit_document(
        &mut self,
        owner: AgentOwner,
        origin: String,
        group: crate::browsing_groups::GroupToken,
    ) -> Result<(), String> {
        if self.members.contains_key(&owner) {
            return Err("document already has cluster membership".into());
        }
        if let Some((id, cluster)) = self
            .clusters
            .iter_mut()
            .find(|(_, cluster)| cluster.group == Some(group) && cluster.origin == origin)
        {
            if cluster.members.len() >= 4096 {
                return Err("isolated cluster member capacity".into());
            }
            cluster.members.insert(owner.clone());
            self.members.insert(owner, *id);
            return Ok(());
        }
        self.admit_root(owner.clone(), origin)?;
        let id = self.members[&owner];
        self.clusters.get_mut(&id).unwrap().group = Some(group);
        Ok(())
    }
    pub(crate) fn admit_child(
        &mut self,
        parent: &AgentOwner,
        child: AgentOwner,
        origin: &str,
    ) -> Result<(), String> {
        if self.members.contains_key(&child) {
            return Err("agent already has cluster membership".into());
        }
        let id = *self
            .members
            .get(parent)
            .ok_or("parent is not cross-origin isolated")?;
        let cluster = self.clusters.get_mut(&id).ok_or("parent cluster retired")?;
        if cluster.origin != origin {
            return Err("shared backing requires same native origin".into());
        }
        if cluster.members.len() >= 4096 {
            return Err("isolated cluster member capacity".into());
        }
        cluster.members.insert(child.clone());
        self.members.insert(child, id);
        Ok(())
    }
    pub(crate) fn isolated(&self, owner: &AgentOwner) -> bool {
        self.members.contains_key(owner)
    }
    pub(crate) fn retire_page(&mut self, page: oryn_common::v3::PageId) {
        let owners = self
            .members
            .keys()
            .filter(|owner| matches!(owner,AgentOwner::Window{page:owned,..}if *owned==page))
            .cloned()
            .collect::<Vec<_>>();
        for owner in owners {
            self.retire(&owner);
        }
        self.retire_shared_principal(oryn_js::message_wire::Principal::Window(page.0));
    }
    #[cfg(unix)]
    pub(crate) fn allocate(
        &mut self,
        params: SharedAllocationInputs<'_>,
    ) -> Result<(SharedMemoryDescriptor, std::os::fd::OwnedFd), String> {
        let SharedAllocationInputs {
            owner,
            byte_length,
            maximum_length,
            granularity,
            guard_length,
            wasm,
            memory64,
        } = params;

        let cluster = *self
            .members
            .get(owner)
            .ok_or("shared memory requires cross-origin isolated cluster")?;
        if !(4096..=65536).contains(&granularity) || !granularity.is_power_of_two() {
            return Err("invalid shared memory mapping granularity".into());
        }
        if memory64 && !wasm {
            return Err("memory64 requires Wasm backing".into());
        }
        let growable = maximum_length.is_some();
        let maximum_length = maximum_length.unwrap_or(byte_length);
        if maximum_length < byte_length {
            return Err("shared maximum below initial length".into());
        }
        if !guard_length.is_multiple_of(granularity) || guard_length > 8 * 1024 * 1024 * 1024 {
            return Err("invalid shared guard extent".into());
        }
        if wasm && (!byte_length.is_multiple_of(65536) || !maximum_length.is_multiple_of(65536)) {
            return Err("Wasm shared memory must use whole pages".into());
        }
        let reserved_length = maximum_length
            .max(1)
            .checked_add(granularity - 1)
            .ok_or("shared memory length overflow")?
            / granularity
            * granularity;
        let mapped_length = reserved_length
            .checked_add(granularity)
            .ok_or("shared memory metadata extent overflow")?;
        if self.backings.len() >= 4096
            || mapped_length > 256 * 1024 * 1024
            || mapped_length
                > 512u64
                    .saturating_mul(1024 * 1024)
                    .saturating_sub(self.bytes)
        {
            return Err("shared memory quota exceeded".into());
        }
        let descriptor = SharedMemoryDescriptor {
            token: SharedMemoryToken(random()?),
            byte_length,
            mapped_length,
            maximum_length,
            data_offset: granularity,
            reserved_length,
            guard_length,
            wasm,
            memory64,
            growable,
        };
        if self.backings.contains_key(&descriptor.token) {
            return Err("shared memory identity collision".into());
        }
        let file = anonymous_file(mapped_length)?;
        let mapping = ParentMapping::new(&file, mapped_length)?;
        mapping
            .length()
            .store(byte_length, std::sync::atomic::Ordering::SeqCst);
        let fd = file.try_clone().map_err(|error| error.to_string())?.into();
        self.backings.insert(
            descriptor.token,
            Backing {
                cluster,
                descriptor,
                file: Arc::new(file),
                mapping,
                holders: BTreeMap::from([(owner.clone(), 1)]),
            },
        );
        self.bytes += mapped_length;
        Ok((descriptor, fd))
    }
    #[cfg(unix)]
    pub(crate) fn open(
        &mut self,
        owner: &AgentOwner,
        descriptor: SharedMemoryDescriptor,
    ) -> Result<std::os::fd::OwnedFd, String> {
        let cluster = self
            .members
            .get(owner)
            .ok_or("shared memory recipient is not isolated")?;
        let backing = self
            .backings
            .get_mut(&descriptor.token)
            .ok_or("shared memory capability retired")?;
        if &backing.cluster != cluster || backing.descriptor != descriptor {
            return Err("foreign shared memory cluster or descriptor".into());
        }
        let count = backing.holders.get(owner).copied().unwrap_or(0);
        if count >= 4096 {
            return Err("shared memory holder capacity".into());
        }
        let fd = backing
            .file
            .try_clone()
            .map_err(|error| error.to_string())?
            .into();
        backing.holders.insert(owner.clone(), count + 1);
        Ok(fd)
    }
    /// Native response compensation: a retired principal may already have
    /// released its holder while another recipient keeps the backing alive.
    #[cfg(unix)]
    pub(crate) fn release_response_lease(
        &mut self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
    ) -> Result<(), String> {
        if self
            .backings
            .get(&token)
            .is_some_and(|backing| backing.holders.contains_key(owner))
        {
            self.release(owner, token)?;
        }
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn release(
        &mut self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
    ) -> Result<(), String> {
        let Some(backing) = self.backings.get_mut(&token) else {
            return Ok(());
        };
        let count = backing
            .holders
            .get_mut(owner)
            .ok_or("foreign shared memory holder")?;
        *count -= 1;
        if *count == 0 {
            backing.holders.remove(owner);
        }
        self.reap_backings();
        Ok(())
    }
    #[cfg(unix)]
    fn backing(&self, owner: &AgentOwner, token: SharedMemoryToken) -> Result<&Backing, String> {
        let backing = self
            .backings
            .get(&token)
            .ok_or("shared memory backing retired")?;
        // A physical mapping is itself a capability. Another same-agent realm
        // may retain its wrapper after the allocating document is retired.
        // New allocation/open still requires live cluster membership.
        if !backing.holders.contains_key(owner) {
            return Err("foreign shared memory mapping".into());
        }
        Ok(backing)
    }
    #[cfg(unix)]
    pub(crate) fn grow_absolute(
        &mut self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
        length: u64,
    ) -> Result<u64, String> {
        let backing = self.backing(owner, token)?;
        let current = backing
            .mapping
            .length()
            .load(std::sync::atomic::Ordering::SeqCst);
        if backing.descriptor.wasm
            || !backing.descriptor.growable
            || length < current
            || length > backing.descriptor.maximum_length
        {
            return Err("invalid absolute shared buffer growth".into());
        }
        backing
            .mapping
            .length()
            .compare_exchange(
                current,
                length,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .map_err(|_| "shared growth raced".to_string())?;
        Ok(current)
    }
    #[cfg(unix)]
    pub(crate) fn grow_pages(
        &mut self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
        pages: u64,
        maximum_pages: u64,
    ) -> Result<u64, String> {
        let backing = self.backing(owner, token)?;
        let current = backing
            .mapping
            .length()
            .load(std::sync::atomic::Ordering::SeqCst);
        let length = pages
            .checked_mul(65536)
            .and_then(|delta| current.checked_add(delta))
            .ok_or("Wasm shared growth overflow")?;
        if !backing.descriptor.wasm
            || current % 65536 != 0
            || length > backing.descriptor.maximum_length
            || length / 65536 > maximum_pages
        {
            return Err("invalid Wasm shared growth".into());
        }
        backing
            .mapping
            .length()
            .compare_exchange(
                current,
                length,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .map_err(|_| "Wasm shared growth raced".to_string())?;
        Ok(current / 65536)
    }
    #[cfg(unix)]
    pub(crate) fn wait_enqueue(
        &mut self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
        offset: u64,
        width64: bool,
        expected: u64,
        timeout_ns: Option<u64>,
    ) -> Result<oryn_js::shared_memory::SharedWaitResult, String> {
        use oryn_js::shared_memory::SharedWaitResult;
        let backing = self.backing(owner, token)?;
        let width = if width64 { 8 } else { 4 };
        let length = backing
            .mapping
            .length()
            .load(std::sync::atomic::Ordering::SeqCst);
        if !offset.is_multiple_of(width) || offset.checked_add(width).is_none_or(|end| end > length)
        {
            return Err("shared wait offset or alignment".into());
        }
        let pointer = unsafe {
            backing
                .mapping
                .address
                .as_ptr()
                .add((backing.descriptor.data_offset + offset) as usize)
        };
        let actual = if width64 {
            unsafe {
                std::sync::atomic::AtomicU64::from_ptr(pointer.cast())
                    .load(std::sync::atomic::Ordering::SeqCst)
            }
        } else {
            (unsafe {
                std::sync::atomic::AtomicU32::from_ptr(pointer.cast())
                    .load(std::sync::atomic::Ordering::SeqCst)
            }) as u64
        };
        if actual
            != if width64 {
                expected
            } else {
                expected as u32 as u64
            }
        {
            return Ok(SharedWaitResult::NotEqual);
        }
        if timeout_ns == Some(0) {
            return Ok(SharedWaitResult::TimedOut);
        }
        let id = self.waits.enqueue(
            owner,
            token,
            offset,
            timeout_ns.map(std::time::Duration::from_nanos),
        )?;
        Ok(SharedWaitResult::Pending(id))
    }
    #[cfg(unix)]
    pub(crate) fn notify(
        &self,
        owner: &AgentOwner,
        token: SharedMemoryToken,
        offset: u64,
        count: u64,
    ) -> Result<u64, String> {
        let backing = self.backing(owner, token)?;
        let length = backing
            .mapping
            .length()
            .load(std::sync::atomic::Ordering::SeqCst);
        if !offset.is_multiple_of(4) || offset.checked_add(4).is_none_or(|end| end > length) {
            return Err("shared notify offset or alignment".into());
        }
        Ok(self.waits.notify(token, offset, count))
    }
    #[cfg(unix)]
    pub(crate) fn wait_result(
        &mut self,
        owner: &AgentOwner,
        id: oryn_js::shared_memory::SharedWaitId,
        cancel: bool,
    ) -> Result<oryn_js::shared_memory::SharedWaitResult, String> {
        use crate::shared_wait::WaitOutcome;
        use oryn_js::shared_memory::SharedWaitResult;
        let outcome = if cancel {
            self.waits.cancel(owner, id)?
        } else {
            self.waits.acknowledge(owner, id)?
        };
        self.reap_backings();
        Ok(match outcome {
            WaitOutcome::Pending => SharedWaitResult::Pending(id),
            WaitOutcome::Ok => SharedWaitResult::Ok,
            WaitOutcome::TimedOut => SharedWaitResult::TimedOut,
            WaitOutcome::Cancelled => SharedWaitResult::Cancelled,
        })
    }
    pub(crate) fn retire(&mut self, owner: &AgentOwner) {
        #[cfg(unix)]
        self.retire_wait_admissions(owner);
        self.waits.retire(owner);
        self.retire_uncommitted_handoffs(owner);
        // Do not erase physical holders here. The V8 backing destruction
        // callback releases them, or actual page/process teardown force-clears
        // its native principal after ordered incoming events have drained.
        if let Some(id) = self.members.remove(owner)
            && let Some(cluster) = self.clusters.get_mut(&id)
        {
            cluster.members.remove(owner);
        }
        #[cfg(unix)]
        self.reap_backings();
        #[cfg(not(unix))]
        self.clusters
            .retain(|_, cluster| !cluster.members.is_empty());
    }
}
#[cfg(unix)]
fn anonymous_file(length: u64) -> Result<std::fs::File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let suffix = random()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = std::ffi::CString::new(format!("/oryn-{}", &suffix[..24]))
        .map_err(|error| error.to_string())?;
    let fd = unsafe {
        libc::shm_open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if unsafe { libc::shm_unlink(name.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    file.set_len(length).map_err(|error| error.to_string())?;
    Ok(file)
}

#[cfg(all(test, unix))]
mod mapping_test_support {
    use super::*;

    pub(super) fn page_granularity() -> u64 {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(page > 0, "native page size must be available");
        let page = page as u64;
        assert!((4096..=65536).contains(&page) && page.is_power_of_two());
        page
    }

    // These tests use sequential MAP_SHARED byte accesses, matching the
    // production descriptor consumer. POSIX shm descriptors need not support
    // pread/pwrite on Darwin. No Rust references alias the shared bytes.
    pub(super) struct Bytes(ParentMapping);

    impl Bytes {
        pub(super) fn new(file: &std::fs::File, length: u64) -> Self {
            Self(ParentMapping::new(file, length).expect("shared mmap"))
        }

        pub(super) fn from_descriptor(
            fd: std::os::fd::OwnedFd,
            descriptor: SharedMemoryDescriptor,
        ) -> Self {
            let file = std::fs::File::from(fd);
            assert_eq!(file.metadata().unwrap().len(), descriptor.mapped_length);
            assert_eq!(descriptor.data_offset % page_granularity(), 0);
            let mapping = Self::new(&file, descriptor.mapped_length);
            drop(file); // The mapping, rather than an open fd, now owns its view.
            mapping
        }

        fn offset(&self, offset: u64, count: usize) -> usize {
            let offset = usize::try_from(offset).unwrap();
            assert!(offset.checked_add(count).unwrap() <= self.0.length);
            offset
        }

        pub(super) fn read<const N: usize>(&self, offset: u64) -> [u8; N] {
            let offset = self.offset(offset, N);
            std::array::from_fn(|index| unsafe {
                self.0.address.as_ptr().add(offset + index).read_volatile()
            })
        }

        pub(super) fn write(&self, offset: u64, bytes: &[u8]) {
            let offset = self.offset(offset, bytes.len());
            for (index, byte) in bytes.iter().enumerate() {
                unsafe {
                    self.0
                        .address
                        .as_ptr()
                        .add(offset + index)
                        .write_volatile(*byte);
                }
            }
        }

        pub(super) fn byte_length(&self) -> u64 {
            self.0.length().load(std::sync::atomic::Ordering::SeqCst)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn anonymous_backing_fits_darwin_name_limit_and_has_requested_length() {
        use mapping_test_support::{Bytes, page_granularity};
        let requested = 4096;
        let file = anonymous_file(requested).expect("portable anonymous shared backing");
        // Darwin reports the physical shm extent rounded to its host page;
        // the requested byte range remains exactly the range tested below.
        let physical = if cfg!(target_vendor = "apple") {
            requested.div_ceil(page_granularity()) * page_granularity()
        } else {
            requested
        };
        assert_eq!(file.metadata().unwrap().len(), physical);
        let mapping = Bytes::new(&file, requested);
        assert_eq!(mapping.read::<1>(0), [0]);
        assert_eq!(mapping.read::<1>(requested - 1), [0]);
        mapping.write(0, &[17]);
        mapping.write(requested - 1, &[29]);
        assert_eq!(mapping.read::<1>(0), [17]);
        assert_eq!(mapping.read::<1>(requested - 1), [29]);
        use std::os::fd::AsRawFd;
        assert_ne!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }
    use oryn_js::worker_wire::AgentId;
    #[test]
    fn actual_shared_descriptor_preserves_bytes_and_native_cluster_isolation() {
        use mapping_test_support::{Bytes, page_granularity};
        let first = AgentOwner::Agent(AgentId(1));
        let child = AgentOwner::Agent(AgentId(2));
        let foreign = AgentOwner::Agent(AgentId(3));
        let mut registry = SharedMemoryRegistry::default();
        registry
            .admit_root(first.clone(), "origin-a".into())
            .unwrap();
        registry
            .admit_child(&first, child.clone(), "origin-a")
            .unwrap();
        registry
            .admit_root(foreign.clone(), "origin-a".into())
            .unwrap();
        assert!(
            registry
                .admit_child(&first, AgentOwner::Agent(AgentId(4)), "origin-b")
                .is_err()
        );
        let (descriptor, fd) = registry
            .allocate(SharedAllocationInputs {
                owner: &first,
                byte_length: 4,
                maximum_length: None,
                granularity: page_granularity(),
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        assert!(
            registry.open(&foreign, descriptor).is_err(),
            "same origin does not imply same agent cluster"
        );
        let mut forged = descriptor;
        forged.byte_length += 1;
        assert!(registry.open(&child, forged).is_err());
        let mapped = registry.open(&child, descriptor).unwrap();
        let first_mapping = Bytes::from_descriptor(fd, descriptor);
        let child_mapping = Bytes::from_descriptor(mapped, descriptor);
        first_mapping.write(descriptor.data_offset, &[1, 2, 3, 4]);
        let bytes = child_mapping.read::<4>(descriptor.data_offset);
        assert_eq!(bytes, [1, 2, 3, 4]);
        child_mapping.write(descriptor.data_offset + 3, &[7]);
        assert_eq!(
            first_mapping.read::<4>(descriptor.data_offset),
            [1, 2, 3, 7]
        );
        registry.release(&first, descriptor.token).unwrap();
        assert!(registry.backings.contains_key(&descriptor.token));
        registry.retire(&child);
        assert!(registry.backings.contains_key(&descriptor.token));
        assert!(registry.open(&child, descriptor).is_err());
        registry.release(&child, descriptor.token).unwrap();
        assert!(registry.backings.is_empty());
        assert_eq!(registry.bytes, 0);
        first_mapping.write(descriptor.data_offset, &[9]);
        let bytes = child_mapping.read::<1>(descriptor.data_offset);
        assert_eq!(
            bytes[0], 9,
            "retained mappings own shared memory independently of future capability resolution"
        );
        assert!(registry.open(&first, descriptor).is_err());
    }
    #[test]
    fn repeated_maps_keep_exact_holder_counts_beyond_document_retirement() {
        let owner = AgentOwner::Agent(AgentId(1));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        let (descriptor, _) = registry
            .allocate(SharedAllocationInputs {
                owner: &owner,
                byte_length: 0,
                maximum_length: None,
                granularity: 16384,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let _second = registry.open(&owner, descriptor).unwrap();
        registry.release(&owner, descriptor.token).unwrap();
        assert!(registry.backings.contains_key(&descriptor.token));
        registry.retire(&owner);
        assert!(registry.backings.contains_key(&descriptor.token));
        registry.release(&owner, descriptor.token).unwrap();
        assert!(registry.backings.is_empty());
        assert!(!registry.isolated(&owner));
        assert!(
            registry
                .allocate(SharedAllocationInputs {
                    owner: &owner,
                    byte_length: 1,
                    maximum_length: None,
                    granularity: 16384,
                    guard_length: 0,
                    wasm: false,
                    memory64: false
                })
                .is_err()
        );
    }
    #[test]
    fn page_retirement_revokes_root_frame_and_cached_generations_without_revoking_peer() {
        use oryn_common::v3::PageId;
        use oryn_js::message_wire::WindowTarget;
        let root = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let frame = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Frame("frame".into()),
            generation: 2,
            epoch: 0,
        };
        let cached = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Root,
            generation: 3,
            epoch: 0,
        };
        let peer = AgentOwner::Window {
            page: PageId(2),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(root.clone(), "origin".into()).unwrap();
        for member in [&frame, &cached, &peer] {
            registry
                .admit_child(&root, member.clone(), "origin")
                .unwrap();
        }
        let (descriptor, _) = registry
            .allocate(SharedAllocationInputs {
                owner: &root,
                byte_length: 4,
                maximum_length: None,
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let _peer = registry.open(&peer, descriptor).unwrap();
        registry.retire_page(PageId(1));
        for member in [&root, &frame, &cached] {
            assert!(!registry.isolated(member));
        }
        assert!(registry.isolated(&peer));
        assert!(registry.open(&root, descriptor).is_err());
        assert!(registry.open(&peer, descriptor).is_ok());
        registry.retire_page(PageId(2));
        assert!(registry.backings.is_empty());
        assert!(registry.clusters.is_empty());
    }
}

#[cfg(unix)]
struct ParentMapping {
    address: std::ptr::NonNull<u8>,
    length: usize,
}
// The mapping exposes only atomic operations and native byte addresses; no
// Rust slice or mutable reference to concurrently shared author data is formed.
#[cfg(unix)]
unsafe impl Send for ParentMapping {}
#[cfg(unix)]
unsafe impl Sync for ParentMapping {}
#[cfg(unix)]
impl ParentMapping {
    fn new(file: &std::fs::File, length: u64) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        let length = usize::try_from(length).map_err(|_| "shared mapping address width")?;
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self {
            address: std::ptr::NonNull::new(address.cast()).ok_or("null shared mapping")?,
            length,
        })
    }
    fn length(&self) -> &std::sync::atomic::AtomicU64 {
        unsafe { std::sync::atomic::AtomicU64::from_ptr(self.address.as_ptr().cast()) }
    }
}
#[cfg(unix)]
impl Drop for ParentMapping {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address.as_ptr().cast(), self.length);
        }
    }
}

#[cfg(all(test, unix))]
mod growth_wait_tests {
    use super::*;
    use oryn_js::{shared_memory::SharedWaitResult, worker_wire::AgentId};
    #[test]
    fn retired_document_mapping_can_grow_wait_and_notify_but_cannot_open_or_allocate() {
        let owner = AgentOwner::Agent(AgentId(71));
        let foreign = AgentOwner::Agent(AgentId(72));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        let (descriptor, _fd) = registry
            .allocate(SharedAllocationInputs {
                owner: &owner,
                byte_length: 4,
                maximum_length: Some(8),
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        registry.retire(&owner);
        assert!(registry.open(&owner, descriptor).is_err());
        assert!(
            registry
                .allocate(SharedAllocationInputs {
                    owner: &owner,
                    byte_length: 4,
                    maximum_length: None,
                    granularity: 4096,
                    guard_length: 0,
                    wasm: false,
                    memory64: false
                })
                .is_err()
        );
        assert!(
            registry
                .grow_absolute(&foreign, descriptor.token, 8)
                .is_err()
        );
        registry.grow_absolute(&owner, descriptor.token, 8).unwrap();
        let SharedWaitResult::Pending(wait) = registry
            .wait_enqueue(&owner, descriptor.token, 0, false, 0, None)
            .unwrap()
        else {
            panic!("wait must enqueue")
        };
        assert_eq!(registry.notify(&owner, descriptor.token, 0, 1).unwrap(), 1);
        assert_eq!(
            registry.wait_result(&owner, wait, false).unwrap(),
            SharedWaitResult::Ok
        );
        registry.release(&owner, descriptor.token).unwrap();
        assert!(registry.backings.is_empty());
    }
    #[test]
    fn native_growth_is_zeroed_and_wait_compare_precedes_zero_timeout() {
        use mapping_test_support::{Bytes, page_granularity};
        let owner = AgentOwner::Agent(AgentId(7));
        let child = AgentOwner::Agent(AgentId(8));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_child(&owner, child.clone(), "origin")
            .unwrap();
        let (descriptor, fd) = registry
            .allocate(SharedAllocationInputs {
                owner: &owner,
                byte_length: 8,
                maximum_length: Some(16),
                granularity: page_granularity(),
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let owner_mapping = Bytes::from_descriptor(fd, descriptor);
        let other = registry.open(&child, descriptor).unwrap();
        let child_mapping = Bytes::from_descriptor(other, descriptor);
        // Keep the wait word zero while proving growth preserves earlier data.
        owner_mapping.write(descriptor.data_offset + 4, &[1, 2, 3, 4]);
        assert_eq!(owner_mapping.byte_length(), 8);
        assert_eq!(child_mapping.byte_length(), 8);
        assert_eq!(
            registry
                .grow_absolute(&owner, descriptor.token, 16)
                .unwrap(),
            8
        );
        let bytes = child_mapping.read::<8>(descriptor.data_offset + 8);
        assert_eq!(bytes, [0; 8]);
        assert_eq!(owner_mapping.byte_length(), 16);
        assert_eq!(child_mapping.byte_length(), 16);
        assert_eq!(
            child_mapping.read::<8>(descriptor.data_offset),
            [0, 0, 0, 0, 1, 2, 3, 4]
        );
        child_mapping.write(descriptor.data_offset + 15, &[7]);
        assert_eq!(owner_mapping.read::<1>(descriptor.data_offset + 15), [7]);
        assert!(registry.grow_absolute(&child, descriptor.token, 8).is_err());
        assert_eq!(
            registry
                .wait_enqueue(&owner, descriptor.token, 0, false, 1, Some(0))
                .unwrap(),
            SharedWaitResult::NotEqual
        );
        assert_eq!(
            registry
                .wait_enqueue(&owner, descriptor.token, 0, false, 0, Some(0))
                .unwrap(),
            SharedWaitResult::TimedOut
        );
        let SharedWaitResult::Pending(id) = registry
            .wait_enqueue(&owner, descriptor.token, 0, false, 0, None)
            .unwrap()
        else {
            panic!("pending waiter")
        };
        registry.release(&owner, descriptor.token).unwrap();
        assert_eq!(registry.notify(&child, descriptor.token, 0, 1).unwrap(), 1);
        registry.release(&child, descriptor.token).unwrap();
        assert!(
            registry.backings.contains_key(&descriptor.token),
            "terminal waiter retains backing until acknowledgement"
        );
        assert_eq!(
            registry.wait_result(&owner, id, false).unwrap(),
            SharedWaitResult::Ok
        );
        assert!(registry.backings.is_empty());
    }
    #[test]
    fn concurrent_wasm_delta_growth_returns_distinct_previous_page_lengths() {
        let owner = AgentOwner::Agent(AgentId(9));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        let (descriptor, _fd) = registry
            .allocate(SharedAllocationInputs {
                owner: &owner,
                byte_length: 65536,
                maximum_length: Some(3 * 65536),
                granularity: 4096,
                guard_length: 0,
                wasm: true,
                memory64: false,
            })
            .unwrap();
        let registry = Arc::new(std::sync::Mutex::new(registry));
        assert!(
            registry
                .lock()
                .unwrap()
                .grow_pages(&owner, descriptor.token, 1, 1)
                .is_err(),
            "call-site maximum further restricts immutable descriptor maximum"
        );
        let mut threads = Vec::new();
        for _ in 0..2 {
            let registry = registry.clone();
            let owner = owner.clone();
            threads.push(std::thread::spawn(move || {
                registry
                    .lock()
                    .unwrap()
                    .grow_pages(&owner, descriptor.token, 1, 3)
                    .unwrap()
            }));
        }
        let mut previous = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        previous.sort();
        assert_eq!(previous, vec![1, 2]);
        assert!(
            registry
                .lock()
                .unwrap()
                .grow_pages(&owner, descriptor.token, 1, 3)
                .is_err()
        );
    }
}

#[cfg(unix)]
impl SharedMemoryRegistry {
    pub(crate) fn execution_placements(&self) -> Vec<serde_json::Value> {
        self.backings.values().map(|backing|serde_json::json!({
            "backing_sha256":crate::execution_placement::fingerprint(&backing.descriptor.token.0),
            "wasm":backing.descriptor.wasm,"memory64":backing.descriptor.memory64,"growable":backing.descriptor.growable,
            "maximum_length":backing.descriptor.maximum_length,
            "holders":backing.holders.iter().filter(|(_,count)|**count>0).map(|(owner,_)|crate::execution_placement::owner(owner)).collect::<Vec<_>>()
        })).collect()
    }
}
