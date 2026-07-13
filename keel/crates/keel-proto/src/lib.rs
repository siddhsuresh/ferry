//! keel-proto — PTP/MTP wire types and codec. Pure, no I/O.
//! Ported from go-mtpfs mtp/{const,encoding,types}.go per docs/CONTRACTS.md.
pub mod consts;
pub mod container;
pub mod codec;
pub mod datasets;
pub mod error;

pub use consts::*;
pub use container::{Container, ContainerKind, HDR_LEN, MAX_PARAMS};
pub use datasets::*;
pub use error::{ProtoError, RcError};
