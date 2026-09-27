//! Shared code for the probes: the Win32 helpers live in the core crate.

#[cfg(windows)]
pub use eve_chatterer_core::winapi as winutil;
