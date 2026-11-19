use fast_core::document::{Document, Position, Selection};
use fast_core::file;
use fast_core::search::{Options, Search};

#[test]
fn multi_cursor_edits_round_trip_selections_and_content() {
    let mut doc = Document::new("alpha 🌱 beta\r\nend");
    let selections = vec![Selection { anchor: 5, head: 0 }, Selection::caret(11)];
    doc.set_selections(selections.clone());
    doc.insert("!", false);
    assert_eq!(doc.snapshot().to_string(), "! 🌱 !beta\r\nend");
    assert_eq!(
        doc.selections(),
        &[Selection::caret(1), Selection::caret(8)]
    );

    doc.undo();
    assert_eq!(doc.snapshot().to_string(), "alpha 🌱 beta\r\nend");
    assert_eq!(doc.selections(), selections);
    doc.redo();
    assert_eq!(doc.snapshot().to_string(), "! 🌱 !beta\r\nend");
}

#[test]
fn typing_groups_and_saved_revisions_are_stable() {
    let mut doc = Document::default();
    doc.insert("a", true);
    doc.insert("b", true);
    doc.end_typing();
    let saved = doc.revision();
    doc.insert("c", true);
    doc.insert("d", true);
    doc.undo();
    assert_eq!(doc.snapshot().to_string(), "ab");
    assert_eq!(doc.revision(), saved);
    doc.undo();
    assert!(doc.is_empty());
    doc.redo();
    assert_eq!(doc.revision(), saved);
}

#[test]
fn alternate_redo_recovers_an_older_branch() {
    let mut doc = Document::new("branch");
    doc.select(Selection::caret(doc.len()));
    doc.insert(" a", false);
    doc.undo();
    doc.insert(" b", false);
    doc.undo();
    doc.redo_alternate();
    assert_eq!(doc.snapshot().to_string(), "branch a");
    doc.undo();
    doc.redo_alternate();
    assert_eq!(doc.snapshot().to_string(), "branch b");
}

#[test]
fn codecs_validate_boundaries_and_navigation_respects_graphemes() {
    let doc = Document::new("a🌱e\u{301}\r\nβ");
    let at = Position { line: 0, column: 3 };
    assert_eq!(doc.byte_to_utf16(5), Some(at));
    assert_eq!(doc.utf16_to_byte(at), Some(5));
    assert_eq!(doc.utf16_to_byte(Position { column: 2, ..at }), None);
    assert_eq!(doc.byte_to_utf8(6), Some(Position { line: 0, column: 6 }));
    assert_eq!(doc.utf8_to_byte(Position { column: 7, ..at }), None);
    assert_eq!(doc.byte_to_utf16(9), None);
    assert_eq!(doc.next_grapheme(5), 8);
    assert_eq!(doc.previous_grapheme(10), 8);
    assert_eq!(doc.line_range(1), 10..12);
}

#[test]
fn replace_all_is_one_undo_and_respects_whole_words_and_captures() -> Result<(), regex::Error> {
    let mut doc = Document::new("Item 2\nitem 7\nitems 10\n");
    let search = Search::new(
        r"item (\d)",
        Options {
            regex: true,
            whole_word: true,
            ..Options::default()
        },
    )?;
    let before = doc.snapshot().to_string();
    assert_eq!(search.matches(&before).len(), 2);
    doc.replace_ranges(search.replacements(&before, "Result $1", None));
    assert_eq!(doc.snapshot().to_string(), "Result 2\nResult 7\nitems 10\n");
    doc.undo();
    assert_eq!(doc.snapshot().to_string(), before);
    Ok(())
}

#[test]
fn opens_and_saves_twenty_thousand_lines() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("main.typ");
    let text = (0..20_000)
        .map(|line| format!("= Example {line}\r\n"))
        .collect::<String>();
    std::fs::write(&path, &text)?;
    let mut doc = Document::from_rope(file::open(&path)?.text);
    assert_eq!(doc.line_count(), 20_001);
    let end = doc.line_range(10_000).end;
    doc.select(Selection::caret(end));
    doc.insert(" edited", false);
    file::save(&path, &doc.snapshot())?;
    let saved = std::fs::read_to_string(path)?;
    assert_eq!(
        saved,
        text.replacen("= Example 10000\r\n", "= Example 10000 edited\r\n", 1)
    );
    Ok(())
}
