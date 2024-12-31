//! TODO: Crate docs

#![cfg_attr(not(test), no_std)]
#![allow(missing_docs)]

/// TODO: Dap docs
pub mod dap;
pub mod jtag;
pub mod swd;
pub mod swj;
pub mod swo;
pub mod usb;

#[cfg(test)]
mod mock_device;

// Re-export the usb-device crate, so that crates depending on us can use it without
// having to track it as a separate dependency.
pub use usb_device;
