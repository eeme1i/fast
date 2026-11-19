use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use fast_core::document::{Document, Edit};
use fast_theme::tok::CodePalette;
use rune::{Latest, Span, Waker};
use typst_syntax::{LinkedNode, Source, SyntaxKind, Tag, highlight};

#[derive(Clone)]
struct Highlight {
    range: Range<usize>,
    tag: Option<Tag>,
    hover: bool,
}

pub(crate) struct Syntax {
    highlights: Arc<[Highlight]>,
    parser: Arc<Mutex<Parser>>,
    latest: Latest<Parsed>,
    generation: u64,
    pending: Vec<(u64, Vec<Edit>)>,
}

struct Parsed {
    generation: u64,
    highlights: Arc<[Highlight]>,
}

struct Parser {
    source: Source,
    highlights: Vec<Highlight>,
    generation: u64,
}

impl Syntax {
    pub fn new(document: &Document) -> Self {
        let parser = Parser::new(document);
        Self {
            highlights: parser.highlights.clone().into(),
            parser: Arc::new(Mutex::new(parser)),
            latest: Latest::default(),
            generation: 0,
            pending: Vec::new(),
        }
    }

    pub fn set_waker(&self, waker: Waker) {
        self.latest.set_waker(waker);
    }

    pub fn update(&mut self, document: &Document, edits: Vec<Edit>) {
        self.generation += 1;
        self.pending.push((self.generation, edits.clone()));
        let parser = self.parser.clone();
        self.latest.request(
            (
                self.generation,
                Document::from_rope(document.snapshot()),
                edits,
            ),
            move |(generation, document, edits)| {
                let mut parser = parser
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if generation > parser.generation {
                    let contiguous = generation == parser.generation + 1;
                    parser.update(&document, contiguous.then_some(edits.as_slice()));
                    parser.generation = generation;
                }
                Parsed {
                    generation: parser.generation,
                    highlights: parser.highlights.clone().into(),
                }
            },
        );
    }

    pub fn poll(&mut self) -> bool {
        let Some(parsed) = self.latest.take() else {
            return false;
        };
        self.highlights = parsed.highlights;
        self.pending
            .retain(|(generation, _)| *generation > parsed.generation);
        true
    }

    pub fn spans(&self, line: Range<usize>, text: &str, colors: CodePalette) -> Vec<Span> {
        match self.parsed_range(line) {
            Some(line) => paint(&self.highlights, line, colors),
            None => local_spans(text, colors),
        }
    }

    pub fn hover_byte(&self, glyph: usize) -> Option<usize> {
        let parsed = self.parsed_range(glyph..glyph + 1)?.start;
        let index = self
            .highlights
            .partition_point(|span| span.range.end <= parsed);
        let token = self.highlights.get(index)?;
        if token.hover && token.range.contains(&parsed) {
            glyph.checked_sub(parsed - token.range.start)
        } else {
            None
        }
    }

    fn parsed_range(&self, mut range: Range<usize>) -> Option<Range<usize>> {
        for edit in self
            .pending
            .iter()
            .rev()
            .flat_map(|(_, edits)| edits.iter().rev())
        {
            if range.end <= edit.range.start {
                continue;
            }
            if range.start >= edit.range.start + edit.text.len() {
                let delta = edit.range.len() as isize - edit.text.len() as isize;
                range = range.start.saturating_add_signed(delta)
                    ..range.end.saturating_add_signed(delta);
                continue;
            }
            return None;
        }
        Some(range)
    }
}

impl Parser {
    fn new(document: &Document) -> Self {
        let source = Source::detached(document.snapshot().to_string());
        let mut highlights = Vec::new();
        collect(
            &LinkedNode::new(source.root()),
            &(0..document.len()),
            None,
            false,
            &mut highlights,
        );
        Self {
            source,
            highlights,
            generation: 0,
        }
    }

    fn update(&mut self, document: &Document, edits: Option<&[Edit]>) {
        let previous_len = self.source.text().len();
        let dirty = match edits {
            Some([edit]) => self.source.edit(edit.range.clone(), &edit.text),
            _ => self.source.replace(&document.snapshot().to_string()),
        };
        let shift = self.source.text().len() as isize - previous_len as isize;
        let previous_end = dirty.end.saturating_add_signed(-shift);
        let first = self
            .highlights
            .partition_point(|span| span.range.end <= dirty.start);
        let last = self
            .highlights
            .partition_point(|span| span.range.start < previous_end)
            .max(first);
        let mut changed = Vec::new();
        if let Some(span) = self
            .highlights
            .get(first)
            .filter(|span| span.range.start < dirty.start)
        {
            changed.push(Highlight {
                tag: span.tag,
                hover: span.hover,
                range: span.range.start..dirty.start,
            });
        }
        let start = changed.len();
        let from_end = dirty.start > self.source.text().len() - dirty.end;
        collect(
            &LinkedNode::new(self.source.root()),
            &dirty,
            None,
            from_end,
            &mut changed,
        );
        if from_end {
            changed[start..].reverse();
        }
        if let Some(span) = last
            .checked_sub(1)
            .and_then(|index| self.highlights.get(index))
            && last > first
            && span.range.end > previous_end
        {
            changed.push(Highlight {
                tag: span.tag,
                hover: span.hover,
                range: dirty.end..span.range.end.saturating_add_signed(shift),
            });
        }
        for span in &mut self.highlights[last..] {
            span.range.start = span.range.start.saturating_add_signed(shift);
            span.range.end = span.range.end.saturating_add_signed(shift);
        }
        self.highlights.splice(first..last, changed);
    }
}

fn local_spans(text: &str, colors: CodePalette) -> Vec<Span> {
    // Keep pathological single lines out of the input frame. The file parser still colours them.
    if text.len() > 4096 {
        return Vec::new();
    }
    let root = typst_syntax::parse(text);
    let area = 0..text.len();
    let mut spans = Vec::new();
    collect(&LinkedNode::new(&root), &area, None, false, &mut spans);
    paint(&spans, area, colors)
}

fn paint(highlights: &[Highlight], line: Range<usize>, colors: CodePalette) -> Vec<Span> {
    let first = highlights.partition_point(|span| span.range.end <= line.start);
    highlights[first..]
        .iter()
        .take_while(|span| span.range.start < line.end)
        .filter_map(|span| {
            let start = span.range.start.saturating_sub(line.start);
            let end = span.range.end.min(line.end) - line.start;
            let color = match span.tag? {
                Tag::Comment => colors.comment,
                Tag::String | Tag::Raw => colors.string,
                Tag::Keyword | Tag::MathDelimiter | Tag::Operator => colors.keyword,
                Tag::Function | Tag::Interpolated | Tag::Number => colors.code,
                Tag::Heading | Tag::Link | Tag::Label | Tag::Ref => colors.markup,
                Tag::Error => colors.error,
                _ => colors.punctuation,
            };
            Some(Span::new(start, end).color(color))
        })
        .collect()
}

fn collect(
    node: &LinkedNode<'_>,
    area: &Range<usize>,
    inherited: Option<Tag>,
    from_end: bool,
    out: &mut Vec<Highlight>,
) {
    let range = node.range();
    if range.start >= area.end || range.end <= area.start {
        return;
    }
    let own_tag = highlight(node);
    let tag = own_tag.or(inherited);
    if node.get().children().len() == 0 {
        let hover = matches!(
            node.kind(),
            SyntaxKind::Ident
                | SyntaxKind::MathIdent
                | SyntaxKind::Str
                | SyntaxKind::Label
                | SyntaxKind::RefMarker
                | SyntaxKind::Link
                | SyntaxKind::Int
                | SyntaxKind::Float
                | SyntaxKind::Numeric
        ) || own_tag == Some(Tag::Keyword);
        if tag.is_some() || hover {
            let range = range.start.max(area.start)..range.end.min(area.end);
            if !range.is_empty() {
                out.push(Highlight { range, tag, hover });
            }
        }
        return;
    }
    if from_end {
        for child in node
            .children()
            .rev()
            .take_while(|child| child.range().end > area.start)
        {
            collect(&child, area, tag, from_end, out);
        }
    } else {
        for child in node
            .children()
            .take_while(|child| child.offset() < area.end)
        {
            collect(&child, area, tag, from_end, out);
        }
    }
}
