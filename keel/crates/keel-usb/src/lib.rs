//! keel-usb — USB transport over nusb (pure Rust, IOKit). No libusb.
//! Ported from go-mtpfs mtp/{select,mtp}.go usb layers + ganeshrvel/usb call shapes.
pub mod discover;
pub mod pipes;
pub mod handle;

pub use discover::{discover, Discovered, DiscoverError, UsbInfo};
pub use pipes::UsbTransport;
