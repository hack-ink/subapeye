//!

pub use Api as ApiFrontier;

// subapeye
use crate::apeye::api::prelude::*;

///
pub trait Api {}
impl<T> Api for T where T: Layer {}
