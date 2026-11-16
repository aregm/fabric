//! A bounded line reader for stdin: lines end with U+000A, and a line longer than the bound
//! is reported once and skipped without being held in memory (README section 1).

use std::io::{self, BufRead, ErrorKind};

/// One result of [`LineReader::next_line`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A complete line, without its line feed, of at most the bound.
    Frame(Vec<u8>),
    /// A line passed the bound. It is reported as soon as it does; the rest of it, up to and
    /// including its line feed, is skipped.
    Oversized,
    /// The input ended inside a line that had no line feed; the number of bytes it had.
    Unterminated(usize),
    /// The input ended at a line boundary (or inside a line already reported as oversized).
    End,
}

/// Reads lines of at most `limit` bytes. It never holds more than `limit` bytes of a line,
/// besides the fixed buffer of the inner reader.
pub struct LineReader<R> {
    inner: R,
    limit: usize,
    skipping: bool,
}

impl<R: BufRead> LineReader<R> {
    #[must_use]
    pub fn new(inner: R, limit: usize) -> Self {
        LineReader {
            inner,
            limit,
            skipping: false,
        }
    }

    /// The next line. After [`Line::End`] or [`Line::Unterminated`] every call returns
    /// [`Line::End`].
    pub fn next_line(&mut self) -> io::Result<Line> {
        let mut line = Vec::new();
        loop {
            let available = match self.inner.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                self.skipping = false;
                return Ok(if line.is_empty() {
                    Line::End
                } else {
                    Line::Unterminated(line.len())
                });
            }
            let newline = available.iter().position(|&b| b == b'\n');
            let content = newline.unwrap_or(available.len());
            let used = newline.map_or(available.len(), |i| i + 1);
            if self.skipping {
                self.inner.consume(used);
                if newline.is_some() {
                    self.skipping = false;
                }
                continue;
            }
            if line.len() + content > self.limit {
                self.inner.consume(used);
                // Skip the rest of the line unless its line feed was in this chunk.
                self.skipping = newline.is_none();
                return Ok(Line::Oversized);
            }
            line.extend_from_slice(&available[..content]);
            self.inner.consume(used);
            if newline.is_some() {
                return Ok(Line::Frame(line));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    /// Reads `input` through an inner buffer of `capacity` bytes until the end.
    fn read_all(input: &[u8], limit: usize, capacity: usize) -> Vec<Line> {
        let mut reader = LineReader::new(BufReader::with_capacity(capacity, input), limit);
        let mut lines = Vec::new();
        loop {
            let line = reader.next_line().unwrap();
            let end = line == Line::End;
            lines.push(line);
            if end {
                return lines;
            }
        }
    }

    fn frame(text: &str) -> Line {
        Line::Frame(text.as_bytes().to_vec())
    }

    #[test]
    fn splits_at_line_feeds() {
        for capacity in [1, 2, 3, 64] {
            assert_eq!(
                read_all(b"ab\n\ncd\n", 4, capacity),
                [frame("ab"), frame(""), frame("cd"), Line::End]
            );
        }
    }

    #[test]
    fn a_line_at_the_bound_is_a_frame() {
        for capacity in [1, 3, 64] {
            assert_eq!(
                read_all(b"abcd\nx\n", 4, capacity),
                [frame("abcd"), frame("x"), Line::End]
            );
        }
    }

    #[test]
    fn a_longer_line_is_reported_once_and_skipped() {
        for capacity in [1, 2, 5, 64] {
            assert_eq!(
                read_all(b"abcde\nx\nabcdefghijklmnop\ny\n", 4, capacity),
                [
                    Line::Oversized,
                    frame("x"),
                    Line::Oversized,
                    frame("y"),
                    Line::End
                ],
                "capacity {capacity}"
            );
        }
    }

    #[test]
    fn an_unterminated_last_line_is_reported() {
        assert_eq!(
            read_all(b"ab\ncd", 4, 64),
            [frame("ab"), Line::Unterminated(2), Line::End]
        );
    }

    #[test]
    fn end_inside_an_oversized_line_is_the_end() {
        assert_eq!(read_all(b"abcdefgh", 4, 2), [Line::Oversized, Line::End]);
    }

    #[test]
    fn carriage_returns_are_kept() {
        assert_eq!(read_all(b"ab\r\n", 4, 64), [frame("ab\r"), Line::End]);
    }
}
