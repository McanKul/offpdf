//! Isolation layer for on-device inference.
//!
//! Callers use [`InferenceBackend`]. This fold ships only an in-process Fake
//! so cancel, fail, and health can be exercised without a model file. When a
//! real backend is added later, keep the Fake injectable so tests still do
//! not need weights.
//!
//! Generate is UTF-8 prompt in, UTF-8 text out — not a document path and not
//! file bytes. Settings IPC lives in `commands/ai.rs`, not here.

mod fake;
pub(crate) mod lifecycle;
pub mod store;

use crate::error::AppError;

use fake::FakeBackend;

/// Load / unload / generate / cancel / status / health. Methods take `&self`
/// so `cancel` can run from another thread while `generate` is in flight.
///
/// This fold's tests call inherent methods on the Fake handle (no trait import).
#[allow(dead_code)]
pub trait InferenceBackend: Send + Sync {
    fn load(&self) -> Result<(), AppError>;
    fn unload(&self) -> Result<(), AppError>;
    fn generate(&self, prompt: &str) -> Result<String, AppError>;
    fn cancel(&self);
    fn status(&self) -> BackendStatus;
    fn health(&self) -> Health;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStatus {
    Unloaded,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub ok: bool,
    pub backend_id: &'static str,
}

/// Scripted Fake behaviour set at construction. No model file, no download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeScript {
    Unhealthy,
}

fn as_backend<T: InferenceBackend>(backend: T) -> T {
    backend
}

/// In-process Fake. No filesystem path, no download.
///
/// Returns a crate-visible handle with inherent methods so callers (and
/// `ai_tests`) do not need the trait in scope. The concrete type lives in
/// a private submodule and is not re-exported.
pub(crate) fn fake() -> FakeBackend {
    as_backend(FakeBackend::new(None))
}

/// In-process Fake with a scripted health / fail path.
pub(crate) fn fake_with_script(script: FakeScript) -> FakeBackend {
    as_backend(FakeBackend::new(Some(script)))
}
