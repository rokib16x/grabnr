//! grabnr engine: discover network links, bind transfers to them, and measure
//! whether traffic really leaves through the interface we asked for.

pub mod adapt;
pub mod auth;
pub mod bind;
pub mod checksum;
pub mod disk;
pub mod download;
pub mod error;
pub mod fetch;
pub mod ftp;
pub mod hls;
pub mod interfaces;
pub mod limiter;
pub mod links;
pub mod metalink;
pub mod probe;
pub mod remote;
pub mod scheduler;
pub mod sftp;
pub mod spike;
pub mod store;
pub mod writer;

pub use auth::Auth;
pub use bind::{client_for, client_pooled, strength as bind_strength, BindMode, Strength};
pub use checksum::{Algo, Checksum};
pub use download::{download, Event, Options, Route, Snapshot};
pub use error::{Error, Result};
pub use hls::Quality;
pub use interfaces::{list_links, Link, LinkKind};
pub use remote::looks_like as is_remote;
pub use store::Store;
