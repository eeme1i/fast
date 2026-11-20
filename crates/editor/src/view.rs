use std::cell::RefCell;

use fast_core::config::{EditorConfig, EditorFont};
use fast_theme::tok::{self, CodePalette, Palette};
use rune::text::{TextSystem, parley};
use rune::tok::{Family, TextStyle};
use rune::{Align, Cursor, El, GrowthSpec, Shape, canvas, col, el, rich, row, text};

use crate::{BLINK, Host, SemanticKind, VIEWPORT, find, input, selection};

const CODE_PADDING: f32 = 10.0;

thread_local! {
    static GEOMETRY: RefCell<TextSystem> = RefCell::new(TextSystem::default());
}

#[derive(Clone, Copy)]
pub(super) struct Style {
    palette: Palette,
    code: CodePalette,
    scale: f32,
    gutter: f32,
    text: TextStyle,
    pub(super) line_height: f32,
}

impl Style {
    fn code_start(self) -> f32 {
        self.gutter + CODE_PADDING
    }

    fn hairline(self) -> f32 {
        self.scale.recip()
    }
}

pub fn view<S: Host>(
    state: &S,
    ui: fast_core::config::UiConfig,
    config: EditorConfig,
    scale: f32,
) -> El {
    let editor = state.editor();
    let count = editor.displayed_count();
    let style = Style {
        palette: tok::palette(ui.theme).with_font(ui.font_family),
        code: tok::code_palette(ui.theme),
        scale,
        text: TextStyle::new(config.font_size, 400.0)
            .family(match config.font_family {
                EditorFont::SystemMono => Family::SystemMono,
                EditorFont::Mono => Family::Mono,
            })
            .features(if config.ligatures {
                ""
            } else {
                "'liga' 0, 'clig' 0, 'calt' 0"
            })
            .lh(config.line_height - 4.0),
        line_height: config.line_height,
        gutter: (editor.document.line_count().ilog10() as f32 + 1.0).max(3.0) * 8.0 + 18.0,
    };
    let mut pane = col().key("editor-pane").w_full().grow(1.0).min_h(0.0);
    if editor.find.open {
        pane = pane.child(find::bar::<S>(editor, style.palette, ui.font_size));
    }
    pane.child(
        col()
            .key(VIEWPORT)
            .role("textbox")
            .label("Typst source")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .min_w(0.0)
            .scroll_xy()
            .scroll_margin(0.0)
            .on_scroll_offset({
                let scroll = editor.scroll.clone();
                move |position| {
                    scroll.set(position.offset);
                }
            })
            .focusable(true)
            .no_focus_ring()
            .on_key(input::key::<S>)
            .on_layout_tx(|state: &mut S, _, tx| {
                let editor = state.editor_mut();
                if !editor.mounted {
                    editor.mounted = true;
                    editor.focused = true;
                    tx.focus(VIEWPORT, false);
                    tx.wake_after(BLINK);
                }
            })
            .child(
                col()
                    .key("lines")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows(
                        count,
                        style.line_height,
                        |state: &S, index| line_key(state.editor().displayed_line(index)).into(),
                        move |state: &S, index| {
                            line(state, index, state.editor().displayed_line(index), style)
                        },
                    )
                    .virtual_rows_version(editor.language.fold_version)
                    .on_visible_rows(|state: &mut S, visible| {
                        let editor = state.editor_mut();
                        editor.visible = editor.displayed_line(visible.start)
                            ..editor.displayed_line(visible.end.saturating_sub(1)) + 1;
                        false
                    }),
            ),
    )
}

pub(crate) fn line_key(line: usize) -> String {
    format!("line-{}", line + 1)
}

fn line<S: Host>(state: &S, display_index: usize, index: usize, style: Style) -> El {
    let editor = state.editor();
    let document = &editor.document;
    let range = document.line_range(index);
    let source = document.slice(range.clone());
    let active = document.line_at(document.selection().head) == index;
    let primary = document.selection();
    let diagnostics: Vec<_> = editor
        .language
        .problems
        .iter()
        .filter(|problem| problem.range.start <= range.end && problem.range.end >= range.start)
        .collect();
    let mut line = row()
        .key(line_key(index))
        .h(style.line_height)
        .w_full()
        .center_y()
        .cursor(Cursor::Text)
        .on_drag(move |state: &mut S, drag, tx| {
            selection::pointer(state, display_index, style, drag, tx)
        })
        .child(
            row()
                .key("gutter")
                .w(style.gutter)
                .h_full()
                .pr(8.0)
                .center_y()
                .fill(style.palette.chrome)
                .border_sides([0.0, style.hairline(), 0.0, 0.0], style.palette.divider)
                .child(
                    text((index + 1).to_string())
                        .font(style.text)
                        .w_full()
                        .align(Align::End)
                        .color(if active {
                            style.palette.text
                        } else {
                            style.palette.muted_text
                        })
                        .a11y_hidden(),
                ),
        );
    if active {
        line = line.fill(style.code.active_line);
    }
    if editor.language.track {
        line = line
            .on_pointer_move_local(move |state: &mut S, pointer, tx| {
                let editor = state.editor();
                if !editor.language.track || editor.drag_origin.is_some() {
                    tx.live_only();
                    return;
                }
                let byte = hit(state, index, style, pointer.x)
                    .1
                    .and_then(|glyph| editor.syntax.hover_byte(glyph));
                state.editor_mut().hover_line = Some(index);
                state.editor_hover(byte, Some(pointer.window()), tx);
            })
            .on_hover_change(move |state: &mut S, hovered, tx| {
                if !hovered && state.editor().hover_line == Some(index) {
                    state.editor_mut().hover_line = None;
                    state.editor_hover(None, None, tx);
                } else {
                    tx.live_only();
                }
            });
    }
    if let Some(problem) = diagnostics.first() {
        let color = if diagnostics.iter().any(|problem| problem.error) {
            style.palette.error_text
        } else {
            style.code.keyword
        };
        line = line.child(
            el().key("problem-marker")
                .absolute(2.0, 7.0)
                .size(3.0, 8.0)
                .fill(color)
                .help(&problem.message),
        );
    }
    let collapsed = editor
        .language
        .collapsed
        .iter()
        .any(|fold| fold.start == index);
    if collapsed || editor.language.folds.iter().any(|fold| fold.start == index) {
        line = line.child(
            row()
                .key("fold-toggle")
                .absolute(style.gutter - 8.0, 2.0)
                .size(8.0, style.line_height - 4.0)
                .role("button")
                .label(if collapsed {
                    "Expand lines"
                } else {
                    "Collapse lines"
                })
                .center()
                .hover(style.palette.hover)
                .on_press_tx(move |state: &mut S, tx| {
                    state.editor_mut().toggle_fold(index, tx);
                })
                .child(
                    text(if collapsed { "+" } else { "−" })
                        .font(TextStyle::new(10.0, 500.0))
                        .color(style.palette.muted_text),
                ),
        );
    }

    let first_match = editor
        .find
        .matches
        .partition_point(|matched| matched.end <= range.start);
    let matches = editor.find.matches[first_match..]
        .iter()
        .take_while(|matched| matched.start <= range.end);
    let selected = document
        .selections()
        .iter()
        .map(|selection| selection.range());
    for (kind, ranges, color) in [
        (
            "match",
            matches.cloned().collect::<Vec<_>>(),
            style.code.search,
        ),
        (
            "selection",
            selected.collect::<Vec<_>>(),
            style.code.selection,
        ),
    ] {
        for (i, segment) in ranges.into_iter().enumerate() {
            if segment.is_empty() || segment.start > range.end || segment.end <= range.start {
                continue;
            }
            let from = segment.start.saturating_sub(range.start);
            let to = segment.end.min(range.end) - range.start;
            let rects = with_layout(&source, style, |layout| {
                let start = parley::layout::Cursor::from_byte_index(
                    layout,
                    from,
                    parley::layout::Affinity::Downstream,
                );
                let end = parley::layout::Cursor::from_byte_index(
                    layout,
                    to,
                    parley::layout::Affinity::Upstream,
                );
                let mut rects = parley::layout::Selection::new(start, end)
                    .geometry(layout)
                    .into_iter()
                    .map(|(rect, _)| (rect.x0 as f32, rect.width() as f32))
                    .collect::<Vec<_>>();
                if segment.end > range.end {
                    rects.push((
                        end.geometry(layout, 0.0).x0 as f32,
                        style.text.size * style.scale * 0.6,
                    ));
                }
                rects
            });
            for (j, (x, width)) in rects.into_iter().enumerate() {
                line = line.child(
                    el().key(format!("{kind}-{i}-{j}"))
                        .a11y_hidden()
                        .absolute(style.code_start() + x / style.scale, 2.0)
                        .size(width / style.scale, style.line_height - 4.0)
                        .fill(color),
                );
            }
        }
    }
    let mut spans = editor.syntax.spans(range.clone(), &source, style.code);
    let first = editor
        .language
        .semantic
        .partition_point(|mark| mark.range.end <= range.start);
    for mark in editor.language.semantic[first..]
        .iter()
        .take_while(|mark| mark.range.start < range.end)
    {
        let color = match mark.kind {
            SemanticKind::Keyword => style.code.keyword,
            SemanticKind::String => style.code.string,
            SemanticKind::Comment => style.code.comment,
            SemanticKind::Type => style.code.markup,
            SemanticKind::Symbol => style.code.code,
        };
        spans.push(
            rune::Span::new(
                mark.range.start.saturating_sub(range.start),
                mark.range.end.min(range.end) - range.start,
            )
            .color(color),
        );
    }
    line = line.child(
        rich(source.as_str(), spans)
            .key("source")
            .font(style.text)
            .color(style.palette.text)
            .ml(CODE_PADDING)
            .mr(CODE_PADDING)
            .min_w(1.0),
    );
    if let Some(byte) = state
        .editor_hover_anchor()
        .filter(|byte| range.contains(byte))
    {
        let x = text_x(&source, style, byte - range.start);
        line = line.child(
            el().key(crate::HOVER_ANCHOR)
                .absolute(style.code_start() + x, 0.0)
                .size(1.0, style.line_height)
                .inert()
                .a11y_hidden(),
        );
    }
    for (i, problem) in diagnostics.into_iter().enumerate() {
        let start = problem.range.start.clamp(range.start, range.end) - range.start;
        let end = problem.range.end.clamp(range.start, range.end) - range.start;
        let (left, right) = with_layout(&source, style, |layout| {
            let x = |offset| {
                parley::layout::Cursor::from_byte_index(
                    layout,
                    offset,
                    parley::layout::Affinity::Downstream,
                )
                .geometry(layout, 0.0)
                .x0 as f32
                    / style.scale
            };
            (x(start), x(end))
        });
        let width = (right - left).abs().clamp(3.0, 1400.0);
        let points = (0..=(width / 2.0).ceil() as usize)
            .map(|n| [(2.0 * n as f32).min(width), (n % 2) as f32 * 2.0])
            .collect::<Vec<_>>();
        let color = if problem.error {
            style.palette.error_text
        } else {
            style.code.keyword
        };
        line = line.child(
            canvas(Shape::new().stroke(points, style.hairline(), color))
                .key(format!("diagnostic-{i}"))
                .absolute(
                    style.code_start() + left.min(right),
                    style.line_height - 3.0,
                )
                .size(width, 2.0)
                .help(&problem.message),
        );
    }
    if collapsed {
        line = line.child(
            text("…")
                .key("folded-lines")
                .font(style.text)
                .color(style.palette.accent),
        );
    }
    let first = editor
        .language
        .hints
        .partition_point(|hint| hint.line < index);
    for (i, hint) in editor.language.hints[first..]
        .iter()
        .take_while(|hint| hint.line == index)
        .enumerate()
    {
        line = line.child(
            text(hint.label.as_str())
                .key(format!("inlay-{i}"))
                .font(TextStyle::new(11.0, 400.0).family(Family::SystemMono))
                .color(style.palette.muted_text)
                .px(3.0)
                .fill(style.palette.chrome),
        );
    }

    if editor.focused || active {
        for (i, selection) in document.selections().iter().enumerate() {
            if document.line_at(selection.head) != index {
                continue;
            }
            let relative = selection.head.min(range.end) - range.start;
            let x = text_x(&source, style, relative);
            let mut caret = el()
                .key(format!("caret-{i}"))
                .a11y_hidden()
                .absolute(style.code_start() + x, 2.0)
                .size(style.hairline(), style.line_height - 4.0)
                .fill(style.palette.text)
                .opacity(if editor.focused && editor.blink_on {
                    1.0
                } else {
                    0.0
                });
            if *selection == primary {
                if editor.focused {
                    caret = caret.ime_cursor();
                }
                if !editor.preedit.is_empty() {
                    line = line.child(
                        rich(
                            editor.preedit.as_str(),
                            vec![
                                rune::Span::new(0, editor.preedit.len())
                                    .underline(style.palette.accent),
                            ],
                        )
                        .key("preedit")
                        .absolute(style.code_start() + x, 2.0)
                        .font(style.text)
                        .color(style.palette.text)
                        .fill(style.palette.canvas),
                    );
                }
            }
            line = line.child(caret);
        }
    }
    line
}

pub(super) fn hit<S: Host>(state: &S, line: usize, style: Style, x: f32) -> (usize, Option<usize>) {
    let document = state.editor().document();
    let range = document.line_range(line);
    let source = document.slice(range.clone());
    let local_x = (x - style.code_start()) * style.scale;
    with_layout(&source, style, |layout| {
        let y = 0.5 * style.line_height * style.scale;
        let caret = parley::layout::Cursor::from_point(layout, local_x, y);
        let glyph = parley::layout::Cluster::from_point_exact(layout, local_x, y)
            .map(|(cluster, _)| range.start + cluster.text_range().start);
        (range.start + caret.index(), glyph)
    })
}

fn text_x(source: &str, style: Style, byte: usize) -> f32 {
    with_layout(source, style, |layout| {
        parley::layout::Cursor::from_byte_index(layout, byte, parley::layout::Affinity::Downstream)
            .geometry(layout, 0.0)
            .x0 as f32
            / style.scale
    })
}

fn with_layout<T>(
    source: &str,
    style: Style,
    f: impl FnOnce(&parley::Layout<rune::text::Ink>) -> T,
) -> T {
    GEOMETRY.with_borrow_mut(|text| {
        let layout = text.layout_rich(source, &[], style.text, style.scale, None);
        let result = f(&layout);
        text.end_commit();
        result
    })
}
