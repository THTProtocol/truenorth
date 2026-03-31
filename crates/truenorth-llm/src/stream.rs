//! SSE (Server-Sent Events) stream parser for LLM provider streaming responses.
//!
//! All major LLM providers (Anthropic, OpenAI, Google) deliver streaming responses
//! as Server-Sent Events over an HTTP chunked-transfer connection. This module
//! provides a shared SSE parser that extracts `StreamEvent` values from raw
//! byte streams returned by `reqwest`.
//!
//! ## SSE Format
//!
//! Each event looks like:
//! ```text
//! data: {"type": "content_block_delta", ...}\n\n
//! ```
//!
//! The stream ends with:
//! ```text
//! data: [DONE]\n\n
//! ```
//!
//! Partial JSON chunks may arrive split across multiple HTTP frames. The parser
//! accumulates bytes and emits complete JSON objects only.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;
use tokio_stream::StreamExt;
use tracing::{debug, trace, warn};

/// A parsed SSE line extracted from the raw byte stream.
#[derive(Debug, Clone)]
pub enum SseLine {
    /// A `data:` field with a JSON payload.
    Data(String),
    /// The `[DONE]` sentinel indicating stream completion.
    Done,
    /// An `event:` field specifying the event type.
    EventType(String),
    /// An `id:` field (used for reconnection, ignored here).
    Id(String),
    /// A comment line starting with `:`.
    Comment(String),
    /// An empty line (event boundary in SSE spec).
    Empty,
}

/// Parses a single raw line from an SSE stream into an `SseLine`.
///
/// Leading `"data: "`, `"event: "`, `"id: "`, `": "` prefixes are stripped.
pub fn parse_sse_line(line: &str) -> SseLine {
    let line = line.trim_end_matches('\r');

    if line.is_empty() {
        return SseLine::Empty;
    }
    if line == "data: [DONE]" || line == "data:[DONE]" {
        return SseLine::Done;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        return SseLine::Data(data.to_string());
    }
    if let Some(data) = line.strip_prefix("data:") {
        return SseLine::Data(data.to_string());
    }
    if let Some(event) = line.strip_prefix("event: ") {
        return SseLine::EventType(event.to_string());
    }
    if let Some(event) = line.strip_prefix("event:") {
        return SseLine::EventType(event.to_string());
    }
    if let Some(id) = line.strip_prefix("id: ") {
        return SseLine::Id(id.to_string());
    }
    if let Some(comment) = line.strip_prefix(": ") {
        return SseLine::Comment(comment.to_string());
    }
    if line.starts_with(':') {
        return SseLine::Comment(line[1..].trim().to_string());
    }

    // Unrecognized line — treat as empty
    trace!("Unrecognized SSE line: {:?}", line);
    SseLine::Empty
}

/// Splits a raw SSE byte chunk into individual lines.
///
/// SSE uses `\n` or `\r\n` line endings. This handles both.
pub fn split_sse_chunk(chunk: &str) -> Vec<String> {
    chunk
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect()
}

/// Low-level SSE line extractor.
///
/// Takes a `reqwest` byte stream and emits `SseLine` values.
/// Accumulates partial lines across chunk boundaries.
pub struct SseLineStream<S> {
    inner: S,
    buffer: String,
    finished: bool,
}

impl<S> SseLineStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    /// Creates a new `SseLineStream` wrapping a raw byte stream.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
            finished: false,
        }
    }
}

impl<S> Stream for SseLineStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<SseLine, String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Check if there's a complete line already in the buffer
            if let Some(newline_pos) = self.buffer.find('\n') {
                let line = self.buffer[..newline_pos].to_string();
                self.buffer = self.buffer[newline_pos + 1..].to_string();
                let parsed = parse_sse_line(&line);
                return Poll::Ready(Some(Ok(parsed)));
            }

            if self.finished {
                // Drain any remaining buffer content
                if !self.buffer.is_empty() {
                    let line = std::mem::take(&mut self.buffer);
                    let parsed = parse_sse_line(&line);
                    return Poll::Ready(Some(Ok(parsed)));
                }
                return Poll::Ready(None);
            }

            // Poll the underlying stream for more bytes
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => self.buffer.push_str(&text),
                        Err(e) => {
                            warn!("SSE chunk contained invalid UTF-8: {}", e);
                            // Try lossy conversion and continue
                            let lossy = String::from_utf8_lossy(&bytes.to_vec()).to_string();
                            self.buffer.push_str(&lossy);
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(format!("SSE stream error: {}", e))));
                }
                Poll::Ready(None) => {
                    self.finished = true;
                    // Loop again to drain buffer
                    continue;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Collects raw SSE data lines from a `reqwest` response stream.
///
/// Filters out `[DONE]` sentinel, empty lines, and non-data events,
/// yielding only the raw JSON strings from `data:` fields.
pub async fn collect_sse_data_lines(
    response: reqwest::Response,
) -> impl Stream<Item = Result<String, String>> {
    let byte_stream = response.bytes_stream();
    let line_stream = SseLineStream::new(byte_stream);

    line_stream.filter_map(|result| {
        match result {
            Ok(SseLine::Data(data)) => {
                trace!("SSE data line: {}", data);
                Some(Ok(data))
            }
            Ok(SseLine::Done) => {
                debug!("SSE stream [DONE] sentinel received");
                None
            }
            Ok(SseLine::Empty) | Ok(SseLine::Comment(_)) | Ok(SseLine::Id(_)) => None,
            Ok(SseLine::EventType(t)) => {
                trace!("SSE event type: {}", t);
                None
            }
            Err(e) => Some(Err(e)),
        }
    })
}

/// Attempts to parse a JSON string, returning `None` if it fails.
///
/// Used to handle cases where an SSE data line is not valid JSON
/// (e.g., intermediate chunk that is not yet complete). In practice
/// most providers send complete JSON per event, but this is defensive.
pub fn try_parse_json(data: &str) -> Option<serde_json::Value> {
    match serde_json::from_str(data) {
        Ok(v) => Some(v),
        Err(e) => {
            debug!("Failed to parse SSE JSON chunk (may be partial): {} — data: {:?}", e, data);
            None
        }
    }
}

/// Extracts a string field from a JSON value, returning an empty string if missing.
pub fn json_str<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

/// Extracts a nested string field from a JSON value path.
///
/// Example: `json_nested_str(&val, &["delta", "text"])` extracts `val.delta.text`.
pub fn json_nested_str<'a>(v: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    let mut current = v;
    for key in path {
        current = current.get(key)?;
    }
    current.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_line_data() {
        let line = r#"data: {"type":"content_block_delta"}"#;
        match parse_sse_line(line) {
            SseLine::Data(s) => assert!(s.contains("content_block_delta")),
            other => panic!("Expected Data, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_sse_line_done() {
        assert!(matches!(parse_sse_line("data: [DONE]"), SseLine::Done));
        assert!(matches!(parse_sse_line("data:[DONE]"), SseLine::Done));
    }

    #[test]
    fn test_parse_sse_line_empty() {
        assert!(matches!(parse_sse_line(""), SseLine::Empty));
        assert!(matches!(parse_sse_line("   "), SseLine::Empty));
    }

    #[test]
    fn test_parse_sse_line_event_type() {
        match parse_sse_line("event: message_start") {
            SseLine::EventType(t) => assert_eq!(t, "message_start"),
            other => panic!("Expected EventType, got {:?}", other),
        }
    }

    #[test]
    fn test_split_sse_chunk() {
        let chunk = "data: line1\ndata: line2\n\n";
        let lines = split_sse_chunk(chunk);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "data: line1");
        assert_eq!(lines[1], "data: line2");
        assert_eq!(lines[2], "");
    }

    #[test]
    fn test_try_parse_json_valid() {
        let json = r#"{"type":"text","text":"hello"}"#;
        let v = try_parse_json(json);
        assert!(v.is_some());
        assert_eq!(v.unwrap()["type"], "text");
    }

    #[test]
    fn test_try_parse_json_invalid() {
        let not_json = "this is not json";
        assert!(try_parse_json(not_json).is_none());
    }

    #[test]
    fn test_json_nested_str() {
        let v = serde_json::json!({ "delta": { "type": "text_delta", "text": "hello" } });
        assert_eq!(json_nested_str(&v, &["delta", "text"]), Some("hello"));
        assert_eq!(json_nested_str(&v, &["delta", "missing"]), None);
        assert_eq!(json_nested_str(&v, &["missing", "key"]), None);
    }
}
