//! Agent frames in and out, and the bounded lines they travel in (W06 4.3, 2.5).
//!
//! Every frame is read and built through the contract crate: a request is built as JSON and
//! read back with `decode(Entry::AgentRequest, ...)` before it is sent, and an answer is read
//! with `decode(Entry::AgentResponse, ...)`.

use std::io::{self, BufRead};

use fabric_desktop_contracts::agent::{AgentRequest, AgentResponse};
use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::{Entry, Frame, PROTOCOL, Refusal, canonical, decode};
use serde_json::{Value, json};

/// One line read with a bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The bytes of one line, without its line feed. The last line of the input may have no
    /// line feed.
    Line(Vec<u8>),
    /// A line longer than the bound; its rest has been skipped.
    TooLong,
    /// The end of the input.
    End,
}

/// Reads lines of at most `max` bytes (without the line feed) from a buffered reader.
pub struct LineReader<R> {
    inner: R,
    max: usize,
}

impl<R: BufRead> LineReader<R> {
    pub fn new(inner: R, max: usize) -> LineReader<R> {
        LineReader { inner, max }
    }

    /// The underlying reader, whose buffer stays in place.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// The next line.
    pub fn next_line(&mut self) -> io::Result<Line> {
        let mut line = Vec::new();
        let mut too_long = false;
        loop {
            let buffer = match self.inner.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if buffer.is_empty() {
                return Ok(if too_long {
                    Line::TooLong
                } else if line.is_empty() {
                    Line::End
                } else {
                    Line::Line(line)
                });
            }
            let (chunk, done) = match buffer.iter().position(|&b| b == b'\n') {
                Some(at) => (&buffer[..at], Some(at + 1)),
                None => (buffer, None),
            };
            if !too_long {
                if line.len() + chunk.len() > self.max {
                    too_long = true;
                    line = Vec::new();
                } else {
                    line.extend_from_slice(chunk);
                }
            }
            let used = done.unwrap_or(buffer.len());
            self.inner.consume(used);
            if done.is_some() {
                return Ok(if too_long {
                    Line::TooLong
                } else {
                    Line::Line(line)
                });
            }
        }
    }
}

/// Builds the agent request frame of a tool call and reads it with the contract
/// (W06 3.6 step 4). A refusal means the arguments, or the frame's size, are not the
/// contract's.
pub fn request(call_id: &str, tool: ToolName, params: Value) -> Result<AgentRequest, Refusal> {
    let frame = json!({
        "protocol": PROTOCOL,
        "id": call_id,
        "method": tool.as_str(),
        "params": params,
    });
    match decode(Entry::AgentRequest, canonical(&frame).as_bytes())? {
        Frame::AgentRequest(request) => Ok(request),
        // `decode` of the agent request entry gives only agent requests.
        _ => unreachable!("an agent request entry decodes to an agent request"),
    }
}

/// The line that carries a request: its canonical text and a line feed.
#[must_use]
pub fn request_line(request: &AgentRequest) -> String {
    Frame::AgentRequest(request.clone()).to_line()
}

/// Reads one frame from the broker as an agent response.
pub fn response(bytes: &[u8]) -> Result<AgentResponse, Refusal> {
    match decode(Entry::AgentResponse, bytes)? {
        Frame::AgentResponse(response) => Ok(response),
        _ => unreachable!("an agent response entry decodes to an agent response"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::MAX_FRAME_BYTES;

    #[test]
    fn bounded_lines() {
        let input = b"abc\n0123456789\nxy\nlast".to_vec();
        let mut r = LineReader::new(&input[..], 5);
        assert_eq!(r.next_line().unwrap(), Line::Line(b"abc".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::TooLong);
        assert_eq!(r.next_line().unwrap(), Line::Line(b"xy".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::Line(b"last".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::End);
    }

    #[test]
    fn oversized_request_is_refused() {
        let ok = request(
            "call-1",
            ToolName::RequestStatus,
            json!({"request_id": "r"}),
        );
        assert!(ok.is_ok());
        let big = "x".repeat(MAX_FRAME_BYTES);
        let refused = request(
            "call-1",
            ToolName::CalendarAvailability,
            json!({"task": big}),
        );
        assert!(refused.is_err());
    }
}
