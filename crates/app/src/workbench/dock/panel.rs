use fast_theme::tok::{Palette, row_height};
use rune::{El, col, row, text};

use super::{Dock, Tool};
use crate::{
    State, language,
    view::action,
    workbench::{item, project},
};

pub(super) fn tool_panel(state: &State, tool: Tool, palette: Palette, scale: f32) -> El {
    let height = row_height(state.ui.density);
    let dock_key = format!("move-{}", tool.key());
    let mut header = row()
        .key("dock-header")
        .w_full()
        .h(height)
        .px(3.0)
        .center_y()
        .gap(2.0)
        .fill(palette.chrome)
        .border_sides([0.0, 0.0, scale.recip(), 0.0], palette.divider)
        .child(
            text(tool.label())
                .font(palette.control_text(state.ui.font_size.min(13.0)))
                .color(palette.text)
                .grow(1.0)
                .min_w(0.0)
                .ml(4.0)
                .truncate(),
        );
    if tool == Tool::Project {
        header = header
            .child(
                action(
                    "project-choose",
                    "Folder",
                    palette,
                    11.0,
                    true,
                    project::choose_root,
                )
                .h(height - 2.0)
                .help("Choose project folder"),
            )
            .child(
                action(
                    "project-refresh",
                    "↻",
                    palette,
                    11.0,
                    state.workbench.project.root.is_some(),
                    |state, tx| {
                        state.workbench.project.refresh();
                        tx.instant();
                    },
                )
                .h(height - 2.0)
                .help("Refresh project files"),
            );
    }
    header = header
        .child(
            item(
                palette,
                dock_key.clone(),
                "↔",
                height - 2.0,
                false,
                move |state, tx| {
                    let layout = &mut state.workbench.layout;
                    layout.menu = if layout.menu == Some(tool) {
                        None
                    } else {
                        Some(tool)
                    };
                    tx.instant();
                },
            )
            .w(23.0)
            .px(0.0)
            .justify_center()
            .label("Move tool window")
            .help("Move tool window"),
        )
        .child(
            item(
                palette,
                format!("hide-{}", tool.key()),
                "×",
                height - 2.0,
                false,
                move |state, tx| {
                    state.workbench.close(tool, tx);
                },
            )
            .w(22.0)
            .px(0.0)
            .justify_center()
            .label("Hide tool window"),
        );
    let content = match tool {
        Tool::Project => project::tree(state, palette),
        Tool::Outline | Tool::Problems => language::view::tool(state, tool, palette),
        Tool::Search => project::search(state, palette),
        Tool::Compiler => crate::workbench::compiler(state, palette),
        Tool::Preview => unreachable!("preview has its own panel"),
    };
    col()
        .key(format!("tool-{}", tool.key()))
        .grow(1.0)
        .w_full()
        .min_h(0.0)
        .min_w(0.0)
        .fill(palette.canvas)
        .child(header)
        .child(content)
}

pub(in crate::workbench) fn overlay(state: &State, palette: Palette, scale: f32) -> Option<El> {
    let tool = state.workbench.layout.menu?;
    let mut popup = col()
        .key("dock-location")
        .anchored_fit(
            format!("move-{}", tool.key()),
            rune::Side::Bottom,
            rune::Align::End,
            1.0,
        )
        .w(112.0)
        .padding(3.0)
        .gap(2.0)
        .fill(palette.chrome)
        .border(scale.recip(), palette.divider)
        .focus_group()
        .modal_tx(true, |state: &mut State, tx| {
            state.workbench.layout.menu = None;
            tx.instant();
        });
    for dock in Dock::ALL {
        let selected = state.workbench.layout.side(tool, state.ui) == Some(dock);
        popup = popup.child(item(
            palette,
            format!("move-to-{}", dock.key()),
            dock.name(),
            24.0,
            selected,
            move |state, tx| {
                state.workbench.layout.placements[tool as usize] = Some(dock);
                state.workbench.layout.menu = None;
                state.workbench.save_layout();
                tx.instant();
            },
        ));
    }
    Some(popup)
}
