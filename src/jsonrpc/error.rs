//! JSONRPC error collections.

// crates.io
use thiserror::Error as ThisError;

/// Main error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Error {
	#[error(transparent)]
	Generic(#[from] Generic),
	#[error(transparent)]
	Jsonrpc(#[from] Jsonrpc),
	#[error(transparent)]
	Websocket(#[from] Websocket),
}

/// Generic error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Generic {
	#[error("{0:?}")]
	AlmostImpossible(&'static str),
	#[error("[jsonrpc] {0:?}")]
	Plain(String),
	#[error(transparent)]
	Serde(#[from] serde_json::Error),
	#[error(transparent)]
	Timeout(#[from] tokio::time::error::Elapsed),
}
/// Wrap the error with [`Generic::AlmostImpossible`].
pub fn almost_impossible(error: &'static str) -> Generic {
	Generic::AlmostImpossible(error)
}

/// JSONRPC error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Jsonrpc {
	#[error("[jsonrpc] empty batch")]
	EmptyBatch,
	#[error("[jsonrpc] empty response")]
	EmptyResponse,
	#[error("[jsonrpc] exceeded the maximum number of request queue size, {0:?}")]
	ExceededRequestQueueMaxSize(crate::jsonrpc::Id),
	#[error("[jsonrpc] invalid subscription id")]
	InvalidSubscriptionId,
	#[error("[jsonrpc] response error, {0:?}")]
	Response(serde_json::Value),
}

/// Websocket error.
#[allow(missing_docs)]
#[derive(Debug, ThisError)]
pub enum Websocket {
	#[error("[jsonrpc::ws] websocket closed")]
	Closed,
	#[error(transparent)]
	Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),
}
