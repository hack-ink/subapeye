//! Subapeye error collections.

use thiserror::Error as ThisError;

/// Main error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Error {
	#[error(transparent)]
	Quick(#[from] Quick),

	#[error(transparent)]
	Apeye(#[from] Apeye),
	#[error(transparent)]
	Generic(#[from] Generic),
	#[error(transparent)]
	Jsonrpc(#[from] Jsonrpc),
	#[error(transparent)]
	Tokio(#[from] Tokio),
	#[error(transparent)]
	Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),
}

/// An error helper/wrapper to debug/print the error quickly.
#[derive(Debug)]
pub struct Quick(String);
impl std::fmt::Display for Quick {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		std::fmt::Debug::fmt(self, f)
	}
}
impl std::error::Error for Quick {}
/// Wrap the error with [`Quick`].
pub fn quick_err<E>(e: E) -> Quick
where
	E: std::fmt::Debug,
{
	Quick(format!("{e:?}"))
}

/// Api error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Apeye {
	#[error("[apeye] can not find keys of the storage map")]
	KeysNotFound,
	#[error("[apeye] can not find the storage from runtime, {0:?}")]
	StorageNotFound(String),
}

/// Generic error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Generic {
	#[error("{0:?}")]
	AlmostImpossible(&'static str),
	// #[error(transparent)]
	// Codec(#[from] parity_scale_codec::Error),
	#[error(transparent)]
	Serde(#[from] serde_json::Error),
	#[error(transparent)]
	Submetadatan(#[from] submetadatan::Error),
}
/// Wrap the error with [`Generic::AlmostImpossible`].
pub fn almost_impossible(e_msg: &'static str) -> Generic {
	Generic::AlmostImpossible(e_msg)
}

/// JSONRPC error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Jsonrpc {
	#[error("[jsonrpc] empty batch")]
	EmptyBatch,
	#[error("[jsonrpc] exceeded the maximum number of request queue size, {0:?}")]
	ExceededRequestQueueMaxSize(crate::jsonrpc::Id),
	#[error("[jsonrpc] response error, {0:?}")]
	ResponseError(serde_json::Value),
}

/// Tokio error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Tokio {
	#[error(transparent)]
	OneshotRecv(tokio::sync::oneshot::error::RecvError),
	// e.g. https://github.com/tokio-rs/tokio/blob/master/tokio/src/sync/mpsc/error.rs#L12
	#[error("channel closed")]
	ChannelClosed,
	#[error(transparent)]
	Elapsed(tokio::time::error::Elapsed),
}
