/// Shared trusted-input source admission bound.
pub const MAX_NATIVE_POINTER_SOURCES: usize = 16;
pub mod agent;
pub mod basic_shape;
pub mod box_shadow;
pub mod builtin_image;
pub mod canvas_size;
pub mod color;
pub mod error;
pub mod error_mapping;
pub mod formatter;
pub mod http_headers;
pub mod mime_type;
pub mod multipart;
pub mod navigation_request;
pub mod paint_order;
pub mod protocol;
pub mod resolver;
pub mod transport_timing;
pub mod v2;
pub mod v3;
pub mod v4;
pub mod v5;
pub mod v6;
pub mod visibility;
pub mod xml_encoding;

/// Shared parent/worker compatibility pin for the production style pipeline.
/// Keeping this in the V8-free contract crate prevents manifest verification
/// from drifting from the worker implementation.
pub const STYLE_ENGINE_VERSION: &str = "stylo-computed-values-0.20+taffy-grid-v1";

pub mod intent {
    pub mod define_parser;
    pub mod definition;
    pub mod registry;
}

pub mod css_math;

pub mod scrollbar;

pub mod layout_domain;

pub mod visual_viewport;
