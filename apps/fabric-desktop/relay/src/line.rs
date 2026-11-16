//! A line reader with a bound on the length of a line.

use std::io::{self, BufRead};

/// One read of [`LineReader::next_line`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A line, without its line feed.
    Complete(Vec<u8>),
    /// A line longer than the bound. Only the bound plus one byte of it was kept, then dropped;
    /// the rest of it has not been read.
    Oversized,
    /// The input ended inside a line: bytes came after the last line feed.
    Unterminated,
    /// The input ended after a line feed, or was empty.
    End,
}

/// Reads lines ended by U+000A and never holds more than `max + 1` bytes of a line.
pub(crate) struct LineReader<R> {
    input: R,
    max: usize,
    buffer: Vec<u8>,
}

impl<R: BufRead> LineReader<R> {
    pub(crate) fn new(input: R, max: usize) -> LineReader<R> {
        LineReader {
            input,
            max,
            buffer: Vec::new(),
        }
    }

    /// The input, with whatever it has buffered and not yet read.
    pub(crate) fn into_input(self) -> R {
        self.input
    }

    pub(crate) fn next_line(&mut self) -> io::Result<Line> {
        self.buffer.clear();
        loop {
            let available = match self.input.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(if self.buffer.is_empty() {
                    Line::End
                } else {
                    Line::Unterminated
                });
            }
            let room = self.max + 1 - self.buffer.len();
            let window = &available[..available.len().min(room)];
            if let Some(at) = window.iter().position(|&b| b == b'\n') {
                self.buffer.extend_from_slice(&window[..at]);
                self.input.consume(at + 1);
                return Ok(Line::Complete(std::mem::take(&mut self.buffer)));
            }
            let taken = window.len();
            self.buffer.extend_from_slice(window);
            self.input.consume(taken);
            if self.buffer.len() > self.max {
                self.buffer = Vec::new();
                return Ok(Line::Oversized);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn reader(bytes: &[u8], max: usize, capacity: usize) -> LineReader<impl BufRead> {
        LineReader::new(
            BufReader::with_capacity(capacity, Cursor::new(bytes.to_vec())),
            max,
        )
    }

    #[test]
    fn reads_lines_without_their_line_feed() {
        let mut r = reader(b"ab\n\ncd\n", 10, 8192);
        assert_eq!(r.next_line().unwrap(), Line::Complete(b"ab".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::Complete(Vec::new()));
        assert_eq!(r.next_line().unwrap(), Line::Complete(b"cd".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::End);
        assert_eq!(r.next_line().unwrap(), Line::End);
    }

    #[test]
    fn empty_input_is_the_end() {
        assert_eq!(reader(b"", 10, 8).next_line().unwrap(), Line::End);
    }

    #[test]
    fn a_line_of_exactly_the_bound_is_complete() {
        for capacity in [1, 3, 8192] {
            let mut r = reader(b"abcde\nx\n", 5, capacity);
            assert_eq!(r.next_line().unwrap(), Line::Complete(b"abcde".to_vec()));
            assert_eq!(r.next_line().unwrap(), Line::Complete(b"x".to_vec()));
        }
    }

    #[test]
    fn a_line_one_byte_over_the_bound_is_oversized() {
        for capacity in [1, 4, 8192] {
            let mut r = reader(b"abcdef\n", 5, capacity);
            assert_eq!(r.next_line().unwrap(), Line::Oversized);
        }
    }

    #[test]
    fn oversized_stops_reading_at_the_bound_plus_one() {
        let mut input = BufReader::with_capacity(2, Cursor::new(b"abcdefghij\n".to_vec()));
        let mut r = LineReader::new(&mut input, 3);
        assert_eq!(r.next_line().unwrap(), Line::Oversized);
        drop(r);
        let mut rest = Vec::new();
        io::Read::read_to_end(&mut input, &mut rest).unwrap();
        assert_eq!(rest, b"efghij\n");
    }

    #[test]
    fn bytes_after_the_last_line_feed_are_unterminated() {
        let mut r = reader(b"ab\ncd", 10, 8192);
        assert_eq!(r.next_line().unwrap(), Line::Complete(b"ab".to_vec()));
        assert_eq!(r.next_line().unwrap(), Line::Unterminated);
    }
}
