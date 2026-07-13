//! keel-usb — USB transport over nusb (pure Rust, IOKit). No libusb.
pub mod discover;
pub mod pipes;
pub mod handle;

pub use discover::{discover, Discovered, DiscoverError, UsbInfo};
pub use pipes::UsbTransport;
