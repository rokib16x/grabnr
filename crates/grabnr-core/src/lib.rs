//! grabnr engine: discover network links, bind transfers to them, and measure
//! whether traffic really leaves through the interface we asked for.

pub mod bind;
pub mod interfaces;
pub mod spike;

pub use bind::{client_for, BindMode};
pub use interfaces::{list_links, Link, LinkKind};
