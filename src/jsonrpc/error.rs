//! JSONRPC error collections.

// crates.io
use thiserror::Error as ThisError;

/// Main error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Error {
	#[error(transparent)]
	Jsonrpc(#[from] Jsonrpc),
	#[error(transparent)]
	Generic(#[from] Generic),
	// Move to Websocket error?
	#[error(transparent)]
	Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),
}

/// Generic error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Generic {
	#[error("{0:?}")]
	AlmostImpossible(&'static str),
	#[error(transparent)]
	Serde(#[from] serde_json::Error),
}
/// Wrap the error with [`Generic::AlmostImpossible`].
pub fn almost_impossible(e_msg: &'static str) -> Generic {
	Generic::AlmostImpossible(e_msg)
}

/// JSONRPC error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Jsonrpc {
	#[error(transparent)]
	ChannelClosed(#[from] ChannelClosed),
	#[error("[jsonrpc] empty batch")]
	EmptyBatch,
	#[error("[jsonrpc] empty response")]
	EmptyResponse,
	#[error("[jsonrpc] exceeded the maximum number of request queue size, {0:?}")]
	ExceededRequestQueueMaxSize(crate::jsonrpc::Id),
	#[error("[jsonrpc] response error, {0:?}")]
	Response(serde_json::Value),
	#[error(transparent)]
	Timeout(#[from] tokio::time::error::Elapsed),
}

/// Channel closed error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum ChannelClosed {
	#[error("[jsonrpc] messenger channel closed")]
	Messenger,
	#[error("[jsonrpc] notifier channel closed")]
	Notifier,
}
