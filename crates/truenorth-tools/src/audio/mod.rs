//! Audio input infrastructure for TrueNorth.
//!
//! This module defines the [`AudioInputProvider`] trait, which is the
//! architectural slot for voice transcription backends. In v1, no concrete
//! provider is shipped in the default binary. The trait is defined here so
//! that voice is never "bolted on" — it grows into an already-prepared slot.
//!
//! # Enabling Voice
//!
//! Build with `--features voice` to include the `whisper_apr` stub provider.
//! The `whisper-rs` CUDA-accelerated provider is planned for v1.1.
//!
//! # Architecture Decision (Phase 2, Section 1.3)
//!
//! > Define `AudioInputProvider` trait in v1 as part of the complete architecture.
//! > Ship `whisper.apr`-based voice input as an optional `--features voice`
//! > Cargo feature. Not in the default binary.

use async_trait::async_trait;
use thiserror::Error;

/// An audio sample buffer as raw PCM bytes.
pub type AudioBuffer = Vec<u8>;

/// Errors from audio input and transcription operations.
#[allow(missing_docs)]
#[derive(Debug, Error)]
pub enum AudioError {
    /// The audio feature is not enabled in this build.
    #[error("Voice feature is not enabled. Rebuild with --features voice.")]
    FeatureNotEnabled,

    /// The audio device could not be opened.
    #[error("Failed to open audio device: {0}")]
    DeviceError(String),

    /// Transcription failed.
    #[error("Transcription failed: {0}")]
    TranscriptionError(String),

    /// The audio buffer was too short to transcribe.
    #[error("Audio buffer too short: {bytes} bytes (minimum {min_bytes})")]
    BufferTooShort { bytes: usize, min_bytes: usize },
}

/// The transcribed result from an audio buffer.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// The transcribed text.
    pub text: String,
    /// Confidence score (0.0–1.0), if provided by the backend.
    pub confidence: Option<f32>,
    /// The language detected (ISO 639-1 code), if provided.
    pub language: Option<String>,
    /// Duration of the audio that was transcribed, in seconds.
    pub audio_duration_secs: f32,
}

/// Provider trait for audio input and transcription.
///
/// Every voice backend implements this trait. The `Tool` implementation in the
/// voice module wraps an `AudioInputProvider` so the LLM can request
/// transcription via the standard tool call interface.
///
/// # Contract
///
/// - Implementations must be `Send + Sync` (called from async contexts).
/// - `transcribe` must not block the calling thread for longer than
///   `timeout_ms` milliseconds; it should return `Err(AudioError::TranscriptionError)`
///   rather than hanging indefinitely.
/// - If the feature is disabled, `transcribe` must return
///   `Err(AudioError::FeatureNotEnabled)`.
#[async_trait]
pub trait AudioInputProvider: Send + Sync + std::fmt::Debug {
    /// Returns a human-readable name for this provider.
    fn name(&self) -> &str;

    /// Transcribes the given raw audio buffer.
    ///
    /// # Arguments
    /// * `audio` — raw PCM audio bytes (16 kHz, 16-bit mono recommended).
    /// * `timeout_ms` — maximum time to allow for transcription.
    async fn transcribe(
        &self,
        audio: AudioBuffer,
        timeout_ms: u64,
    ) -> Result<TranscriptionResult, AudioError>;

    /// Returns `true` if this provider is available (dependencies installed,
    /// model loaded, etc.).
    async fn is_available(&self) -> bool;
}

/// A stub `AudioInputProvider` that always returns `FeatureNotEnabled`.
///
/// This is the default provider when the `voice` feature is not enabled.
/// It satisfies the trait contract without pulling in any audio dependencies.
#[derive(Debug)]
pub struct StubAudioProvider;

impl StubAudioProvider {
    /// Creates a new stub provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubAudioProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioInputProvider for StubAudioProvider {
    fn name(&self) -> &str {
        "stub"
    }

    async fn transcribe(
        &self,
        _audio: AudioBuffer,
        _timeout_ms: u64,
    ) -> Result<TranscriptionResult, AudioError> {
        Err(AudioError::FeatureNotEnabled)
    }

    async fn is_available(&self) -> bool {
        false
    }
}

/// When the `voice` feature is enabled, this module contains the
/// `whisper.apr`-based provider.
#[cfg(feature = "voice")]
pub mod whisper_apr {
    use super::*;

    /// A `whisper.apr`-backed transcription provider (pure Rust, no C++ FFI).
    ///
    /// This implementation is a stub awaiting integration with the
    /// `whisper-rs` or `whisper_apr` crate. It documents the integration
    /// point without introducing the full dependency.
    #[derive(Debug)]
    pub struct WhisperAprProvider {
        /// Path to the Whisper model file (GGUF format).
        model_path: std::path::PathBuf,
    }

    impl WhisperAprProvider {
        /// Creates a new `WhisperAprProvider` pointing at the given model file.
        pub fn new(model_path: std::path::PathBuf) -> Self {
            Self { model_path }
        }
    }

    #[async_trait]
    impl AudioInputProvider for WhisperAprProvider {
        fn name(&self) -> &str {
            "whisper_apr"
        }

        async fn transcribe(
            &self,
            audio: AudioBuffer,
            _timeout_ms: u64,
        ) -> Result<TranscriptionResult, AudioError> {
            if audio.len() < 256 {
                return Err(AudioError::BufferTooShort {
                    bytes: audio.len(),
                    min_bytes: 256,
                });
            }

            // TODO(v1.1): Replace this stub with actual whisper_apr crate call.
            // The integration point:
            //   let ctx = whisper_apr::WhisperContext::new(&self.model_path)?;
            //   let result = ctx.transcribe_pcm16(&audio)?;
            //   return Ok(TranscriptionResult { text: result.text, ... });

            Err(AudioError::TranscriptionError(
                "whisper_apr integration not yet implemented; this is the stub placeholder"
                    .to_string(),
            ))
        }

        async fn is_available(&self) -> bool {
            self.model_path.exists()
        }
    }
}
