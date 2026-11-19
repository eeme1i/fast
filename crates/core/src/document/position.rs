use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::Document;

/// Zero-based line and column. The column's unit is specified by the conversion method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Document {
    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }

    pub fn line_at(&self, byte: usize) -> usize {
        self.text.byte_to_line(byte.min(self.len()))
    }

    pub fn line_range(&self, line: usize) -> Range<usize> {
        let line = line.min(self.line_count() - 1);
        let start = self.text.line_to_byte(line);
        let mut end = self.text.line_to_byte(line + 1);
        if let Some(last) = self.text.get_line(line).and_then(|line| {
            line.len_chars()
                .checked_sub(1)
                .and_then(|last| line.get_char(last))
        }) && matches!(
            last,
            '\r' | '\n' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        ) {
            end -= last.len_utf8();
            if last == '\n' && end > start && self.text.byte(end - 1) == b'\r' {
                end -= 1;
            }
        }
        start..end
    }

    pub fn ending(&self) -> &'static str {
        if self.line_count() > 1
            && self.text.line_to_byte(1) - self.line_range(0).end == 2
            && self.text.byte(self.text.line_to_byte(1) - 2) == b'\r'
        {
            "\r\n"
        } else {
            "\n"
        }
    }

    pub fn floor_boundary(&self, byte: usize) -> usize {
        self.text
            .char_to_byte(self.text.byte_to_char(byte.min(self.len())))
    }

    pub(super) fn valid_range(&self, range: &Range<usize>) -> bool {
        range.start <= range.end
            && self.floor_boundary(range.start) == range.start
            && self.floor_boundary(range.end) == range.end
    }

    pub fn byte_to_utf16(&self, byte: usize) -> Option<Position> {
        let line = self.line_at(byte);
        let range = self.line_range(line);
        if byte > range.end || self.floor_boundary(byte) != byte {
            return None;
        }
        let base = self.text.char_to_utf16_cu(self.text.line_to_char(line));
        let column = self.text.char_to_utf16_cu(self.text.byte_to_char(byte)) - base;
        Some(Position { line, column })
    }

    pub fn byte_to_utf8(&self, byte: usize) -> Option<Position> {
        let line = self.line_at(byte);
        let range = self.line_range(line);
        (byte <= range.end && self.floor_boundary(byte) == byte).then(|| Position {
            line,
            column: byte - range.start,
        })
    }

    pub fn utf8_to_byte(&self, position: Position) -> Option<usize> {
        if position.line >= self.line_count() {
            return None;
        }
        let range = self.line_range(position.line);
        let byte = range.start.checked_add(position.column)?;
        (byte <= range.end && self.floor_boundary(byte) == byte).then_some(byte)
    }

    pub fn utf16_to_byte(&self, position: Position) -> Option<usize> {
        if position.line >= self.line_count() {
            return None;
        }
        let base = self
            .text
            .char_to_utf16_cu(self.text.line_to_char(position.line));
        let offset = base.checked_add(position.column)?;
        if offset > self.text.len_utf16_cu() {
            return None;
        }
        let character = self.text.utf16_cu_to_char(offset);
        let byte = self.text.char_to_byte(character);
        (self.text.char_to_utf16_cu(character) == offset
            && byte <= self.line_range(position.line).end)
            .then_some(byte)
    }

    pub fn next_grapheme(&self, byte: usize) -> usize {
        let byte = self.floor_boundary(byte);
        let line = self.line_at(byte);
        let start = self.text.line_to_byte(line);
        let text = self.text.line(line).to_string();
        text.grapheme_indices(true)
            .map(|(offset, grapheme)| start + offset + grapheme.len())
            .find(|&end| end > byte)
            .unwrap_or(byte)
    }

    pub fn previous_grapheme(&self, byte: usize) -> usize {
        let byte = self.floor_boundary(byte);
        let line = self.line_at(byte.saturating_sub(1));
        let start = self.text.line_to_byte(line);
        let text = self.text.line(line).to_string();
        text.grapheme_indices(true)
            .map(|(offset, _)| start + offset)
            .take_while(|&start| start < byte)
            .last()
            .unwrap_or(byte)
    }
}
