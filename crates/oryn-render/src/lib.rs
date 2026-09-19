//! Native page capture facade over the cycle-free CPU raster core.
use oryn_common::v4::{PaintSnapshot, PrintSnapshot};
pub use oryn_render_core::{
    BackgroundGradient, BackgroundSource, BackgroundTile, BoxShadow, DisplayList, GradientStop,
    ImageRendering, LinearGradient, MaskComposite, MaskLayer, PaintCommand,
    RENDER_CONTRACT_VERSION, RenderOptions, RenderedArtifact, RendererCapabilities, capabilities,
    stacking,
};
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("invalid print snapshot: {0}")]
    InvalidPrint(String),
    #[error(transparent)]
    Page(#[from] oryn_native::RuntimeError),
    #[error("invalid render scale {0}")]
    InvalidScale(f64),
    #[error("raster would contain {actual} pixels, limit is {limit}")]
    PixelLimit { actual: usize, limit: usize },
    #[error("image {url} could not be decoded: {detail}")]
    ImageDecode { url: String, detail: String },
    #[error("font could not be rendered: {0}")]
    Font(String),
    #[error("output encoding failed: {0}")]
    Encode(String),
}
impl From<oryn_render_core::RenderError> for RenderError {
    fn from(error: oryn_render_core::RenderError) -> Self {
        use oryn_render_core::RenderError as E;
        match error {
            E::InvalidPrint(value) => Self::InvalidPrint(value),
            E::InvalidScale(value) => Self::InvalidScale(value),
            E::PixelLimit { actual, limit } => Self::PixelLimit { actual, limit },
            E::ImageDecode { url, detail } => Self::ImageDecode { url, detail },
            E::Font(value) => Self::Font(value),
            E::Encode(value) => Self::Encode(value),
        }
    }
}
pub fn build_display_list(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<DisplayList, RenderError> {
    oryn_render_core::build_display_list(snapshot, options).map_err(Into::into)
}
pub fn render_png(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    oryn_render_core::render_png(snapshot, options).map_err(Into::into)
}
pub fn render_pdf(
    snapshot: &PaintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    oryn_render_core::render_pdf(snapshot, options).map_err(Into::into)
}
pub fn render_print_pdf(
    snapshot: &PrintSnapshot,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    oryn_render_core::render_print_pdf(snapshot, options).map_err(Into::into)
}
pub async fn screenshot(
    page: &oryn_native::PageHandle,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let snapshot = page.paint_snapshot().await?;
    render_png(&snapshot, options)
}
pub async fn pdf(
    page: &oryn_native::PageHandle,
    options: RenderOptions,
) -> Result<RenderedArtifact, RenderError> {
    let snapshot = page.print_snapshot(Default::default()).await?;
    render_print_pdf(&snapshot, options)
}
