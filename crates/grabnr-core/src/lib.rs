//! grabnr engine: discover network links, bind transfers to them, and measure
//! whether traffic really leaves through the interface we asked for.

pub mod bind;
pub mod download;
pub mod error;
pub mod interfaces;
pub mod probe;
pub mod scheduler;
pub mod spike;
pub mod store;
pub mod writer;

pub use bind::{client_for, client_pooled, BindMode};
pub use download::{download, Event, Options, Route, Snapshot};
pub use error::{Error, Result};
pub use store::Store;
pub use interfaces::{list_links, Link, LinkKind};
