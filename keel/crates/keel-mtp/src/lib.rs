//! keel-mtp — session, transaction engine, the 13 operations + android ext.
//! Ported from go-mtpfs mtp/{mtp,ops,android}.go per docs/CONTRACTS.md.
pub mod transport;
pub mod session;
pub mod transaction;
pub mod ops;
pub mod android;
pub mod error;

pub use error::MtpError;
pub use session::MtpSession;
pub use transport::{Transport, TransportError};
