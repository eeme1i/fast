use std::{collections::BTreeMap, ops::Range};

pub struct Snippet {
    pub text: String,
    pub stops: Vec<Range<usize>>,
}

/// Expand numbered placeholders and choices. Mirror groups initially share the same text.
pub fn snippet(source: &str) -> Snippet {
    let mut parser = Parser {
        input: source,
        text: String::new(),
        stops: BTreeMap::new(),
    };
    parser.part(false, 0);
    let mut stops: Vec<_> = parser
        .stops
        .into_iter()
        .map(|(id, range)| (if id == 0 { u32::MAX } else { id }, range))
        .collect();
    stops.sort_by_key(|(id, _)| *id);
    Snippet {
        text: parser.text,
        stops: stops.into_iter().map(|(_, range)| range).collect(),
    }
}

struct Parser<'a> {
    input: &'a str,
    text: String,
    stops: BTreeMap<u32, Range<usize>>,
}

impl Parser<'_> {
    fn part(&mut self, nested: bool, depth: usize) {
        while let Some(ch) = self.take() {
            match ch {
                '}' if nested => return,
                '\\' if self.input.starts_with(['$', '}', '\\']) => {
                    if let Some(escaped) = self.take() {
                        self.text.push(escaped);
                    }
                }
                '$' if depth < 32 && self.placeholder(depth) => {}
                _ => self.text.push(ch),
            }
        }
    }

    fn placeholder(&mut self, depth: usize) -> bool {
        let before = self.input;
        let braced = self.input.starts_with('{');
        if braced {
            self.input = &self.input[1..];
        }
        let digits = self.input.bytes().take_while(u8::is_ascii_digit).count();
        let Ok(id) = self.input[..digits].parse::<u32>() else {
            self.input = before;
            return false;
        };
        self.input = &self.input[digits..];
        let start = self.text.len();
        let mirror = self.stops.get(&id).cloned();
        if braced {
            match self.take() {
                Some('}') => {}
                Some(':') => self.part(true, depth + 1),
                Some('|') => self.choice(),
                _ => {
                    self.input = before;
                    return false;
                }
            }
        }
        if let Some(range) = mirror {
            self.text.truncate(start);
            self.text.extend_from_within(range);
        } else {
            self.stops.insert(id, start..self.text.len());
        }
        true
    }

    fn choice(&mut self) {
        let mut first = true;
        while let Some(ch) = self.take() {
            match ch {
                '|' if self.input.starts_with('}') => {
                    self.input = &self.input[1..];
                    return;
                }
                ',' => first = false,
                '\\' => {
                    if let Some(escaped) = self.take()
                        && first
                    {
                        self.text.push(escaped);
                    }
                }
                _ if first => self.text.push(ch),
                _ => {}
            }
        }
    }

    fn take(&mut self) -> Option<char> {
        let ch = self.input.chars().next()?;
        self.input = &self.input[ch.len_utf8()..];
        Some(ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_choices_defaults_and_final_tab_stop() {
        let snippet = snippet(r#"func(${2|green,blue|}, ${1:🌱}, $1, \$0)$0"#);
        assert_eq!(snippet.text, "func(green, 🌱, 🌱, $0)");
        assert_eq!(&snippet.text[snippet.stops[0].clone()], "🌱");
        assert_eq!(&snippet.text[snippet.stops[1].clone()], "green");
        assert_eq!(snippet.stops[2], snippet.text.len()..snippet.text.len());
    }
}
