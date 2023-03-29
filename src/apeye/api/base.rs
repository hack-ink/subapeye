//!

pub mod chain;
pub use chain::Api as ApiChain;

pub mod net;
pub use net::Api as ApiNet;

pub mod state;
pub use state::Api as ApiState;

pub use Api as ApiBase;

// subapeye
use crate::apeye::api::prelude::*;

///
pub trait Api: ApiChain + ApiNet + ApiState {}
impl<T> Api for T where T: Layer {}
