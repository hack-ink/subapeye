//! Substrate API client.

#![deny(missing_docs)]
// https://github.com/rust-lang/rust/issues/60551
#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(byte_slice_trim_ascii)]

pub mod prelude {
	//! Subapeye core prelude.

	pub use std::result::Result as StdResult;

	pub use crate::error::{self, Error};

	/// Subapeye's `Result` type.
	pub type Result<T> = StdResult<T, Error>;
}

pub mod apeye;
pub mod error;
pub mod jsonrpc;
