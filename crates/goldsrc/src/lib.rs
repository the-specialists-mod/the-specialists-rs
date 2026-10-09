//! Thin, safe-ish wrappers over the raw engine tables in `goldsrc-sys`.
//!
//! The `server` and `client` features select which engine table this crate wraps, so each
//! cdylib only links the half it needs.

pub use goldsrc_sys as sys;

pub mod msg;
pub mod panic;
pub mod util;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

/// Wrapper for engine-owned globals. The engine runs every game-library callback on one
/// thread, so plain `static` storage behind this is sound as long as nothing spawns threads
/// that touch it.
pub struct Global<T>(pub T);
unsafe impl<T> Sync for Global<T> {}
