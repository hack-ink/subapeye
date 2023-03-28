use thiserror::Error as ThisError;

pub type SubapeyeResult<T> = Result<T, SubapeyeError>;


/// Subapeye error
#[derive(ThisError, Debug)]
pub enum SubapeyeError {
	#[error("Unsupported: {0}")]
	Unsupported(String),
}
