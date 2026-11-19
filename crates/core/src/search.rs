use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::document::Edit;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

pub struct Search {
    pattern: Regex,
    regex: bool,
}

impl Search {
    pub fn new(query: &str, options: Options) -> Result<Self, regex::Error> {
        let query = if options.regex {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        let query = if options.whole_word {
            format!(r"\b{{start-half}}(?:{query})\b{{end-half}}")
        } else {
            query
        };
        let pattern = RegexBuilder::new(&query)
            .case_insensitive(!options.case_sensitive)
            .build()?;
        Ok(Self {
            pattern,
            regex: options.regex,
        })
    }

    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        self.pattern
            .find_iter(text)
            .map(|found| found.range())
            .collect()
    }

    /// Each result is expressed in the original document's coordinates.
    pub fn replacements(
        &self,
        text: &str,
        replacement: &str,
        only: Option<Range<usize>>,
    ) -> Vec<Edit> {
        self.pattern
            .captures_iter(text)
            .filter_map(|captures| {
                let range = captures.get(0)?.range();
                if only.as_ref().is_some_and(|selected| selected != &range) {
                    return None;
                }
                let text = if self.regex {
                    let mut expanded = String::new();
                    captures.expand(replacement, &mut expanded);
                    expanded
                } else {
                    replacement.to_owned()
                };
                Some(Edit { range, text })
            })
            .collect()
    }
}
