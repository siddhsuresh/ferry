//! keel-proto — PTP/MTP wire types and codec. Pure, no I/O.
pub mod consts;
pub mod container;
pub mod codec;
pub mod datasets;
pub mod error;

pub use consts::*;
pub use container::{Container, ContainerKind, HDR_LEN, MAX_PARAMS};
pub use datasets::*;
pub use error::{ProtoError, RcError};
