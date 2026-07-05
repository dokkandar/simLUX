//! Engine error type, serialisable across the Tauri command boundary.
use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("IES parse error: {0}")]
    IesParse(String),
    #[error("DXF parse error: {0}")]
    DxfParse(String),
    #[error("geometry error: {0}")]
    Geometry(String),
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e.to_string())
    }
}

/// Serialise as a plain string so the frontend receives a readable message as
/// the rejected-promise value.
impl Serialize for EngineError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type EngineResult<T> = Result<T, EngineError>;
