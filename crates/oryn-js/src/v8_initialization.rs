//! Complete process-wide V8 setup before concurrent isolate creation.
use std::sync::Once;

static PREPARE_ARRAY_BUFFER_ALLOCATOR: Once = Once::new();

/// Call after `V8::initialize`, inside the embedder's process initialization
/// barrier, before any thread can create an isolate or a default allocator.
///
/// In pinned V8 150, the default isolate group's sandboxed ArrayBuffer backend
/// is initialized lazily without a lock. Racing first default allocators can
/// register PartitionAlloc address pools twice. Creating the wrapper here
/// initializes the group-owned backend; deleting the wrapper does not destroy
/// that backend. The process must not dispose/reinitialize V8 after this call.
pub fn prepare_default_array_buffer_allocator() {
    PREPARE_ARRAY_BUFFER_ALLOCATOR.call_once(|| {
        drop(v8::new_default_allocator());
    });
}
