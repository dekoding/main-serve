/// Response compression middleware.
///
/// Wraps `tower_http::compression::CompressionLayer` for gzip compression.
use tower_http::compression::CompressionLayer;

/// Build a `CompressionLayer` for gzip response compression.
#[must_use]
pub fn build_compression_layer() -> CompressionLayer {
    CompressionLayer::new()
}
