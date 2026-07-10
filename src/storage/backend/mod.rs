/// Storage backend implementations.
pub mod memory;
/// native
pub mod native;

#[cfg(feature = "s3")]
/// s3
pub mod s3;

#[cfg(feature = "azure")]
/// azure
pub mod azure;

#[cfg(feature = "gcs")]
/// gcs
pub mod gcs;
