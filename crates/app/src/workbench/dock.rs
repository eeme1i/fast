mod divider;
mod panel;

use divider::{inner_divider, outer_divider};
pub(super) use panel::overlay;
use panel::tool_panel;

use fast_core::config::{PreviewDock, Sidebar, UiConfig};
use fast_theme::tok::{Palette, row_height};
use rune::{El, Tx, col, row};
use serde::{Deserialize, Serialize};

use super::{Workbench, item, project};
use crate::{State, language, preview};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Tool {
    Project,
    Outline,
    Problems,
    Search,
    Compiler,
    Preview,
}

impl Tool {
    pub const ALL: [Self; 6] = [
        Self::Project,
        Self::Outline,
        Self::Problems,
        Self::Search,
        Self::Compiler,
        Self::Preview,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Outline => "Outline",
            Self::Problems => "Problems",
            Self::Search => "Search",
            Self::Compiler => "Compiler",
            Self::Preview => "Preview",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Outline => "outline",
            Self::Problems => "problems",
            Self::Search => "search",
            Self::Compiler => "compiler",
            Self::Preview => "preview",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Project => "▤",
            Self::Outline => "≡",
            Self::Problems => "!",
            Self::Search => "⌕",
            Self::Compiler => "›_",
            Self::Preview => "▣",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Project => "Project files (Alt+1)",
            Self::Outline => "Document outline (Alt+2)",
            Self::Problems => "Problems (Alt+6)",
            Self::Search => "Find in Files (Ctrl+Shift+F)",
            Self::Compiler => "Compiler output",
            Self::Preview => "Preview (Alt+P)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Dock {
    Left,
    Right,
    Bottom,
}

impl Dock {
    const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Bottom];

    fn name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }
}

pub(super) struct Layout {
    open: [bool; 6],
    pub(super) placements: [Option<Dock>; 6],
    pub(super) ratios: [f32; 3],
    pub(super) weights: [f32; 6],
    viewport: [f32; 2],
    drag: [f32; 2],
    menu: Option<Tool>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            open: [false; 6],
            placements: [None; 6],
            ratios: [0.29, 0.46, 0.39],
            weights: [1.0; 6],
            viewport: [760.0, 408.0],
            drag: [0.0; 2],
            menu: None,
        }
    }
}

impl Layout {
    pub(super) fn width(&self) -> f32 {
        self.viewport[0]
    }

    pub fn open(&mut self, tool: Tool, open: bool) {
        self.open[tool as usize] = open;
        self.menu = None;
    }

    pub fn is_open(&self, tool: Tool) -> bool {
        self.open[tool as usize]
    }

    pub fn reset_sidebar(&mut self) {
        self.placements[Tool::Project as usize] = None;
        self.placements[Tool::Outline as usize] = None;
    }

    fn side(&self, tool: Tool, ui: UiConfig) -> Option<Dock> {
        self.placements[tool as usize].or(match tool {
            Tool::Project | Tool::Outline => Some(match ui.sidebar {
                Sidebar::Left => Dock::Left,
                Sidebar::Right => Dock::Right,
            }),
            Tool::Problems | Tool::Search | Tool::Compiler => Some(Dock::Bottom),
            Tool::Preview => match ui.preview_dock {
                PreviewDock::Left => Some(Dock::Left),
                PreviewDock::Right => Some(Dock::Right),
                PreviewDock::Bottom => Some(Dock::Bottom),
                PreviewDock::Tab => None,
            },
        })
    }

    fn extent(&self, dock: Dock, other: bool) -> f32 {
        if dock == Dock::Bottom {
            return (self.viewport[1] * self.ratios[dock as usize]).max(78.0);
        }
        let available = (self.viewport[0] - 56.0).max(1.0);
        let max = if other {
            available * 0.34
        } else {
            available * 0.62
        };
        (self.viewport[0] * self.ratios[dock as usize]).clamp(0.0, max)
    }
}

impl Workbench {
    pub(crate) fn open(&mut self, tool: Tool) {
        self.layout.open(tool, true);
    }

    pub(crate) fn is_open(&self, tool: Tool) -> bool {
        self.layout.is_open(tool)
    }

    pub(crate) fn close(&mut self, tool: Tool, tx: &mut Tx) {
        self.layout.open(tool, false);
        if tool == Tool::Search {
            self.project.cancel_search();
        }
        tx.focus(fast_editor::VIEWPORT, false);
        tx.instant();
    }
}

pub(super) fn toggle(state: &mut State, tool: Tool, tx: &mut Tx) {
    if tool == Tool::Preview {
        if preview::available(state) {
            preview::toggle(state, tx);
        }
    } else if state.workbench.is_open(tool) {
        state.workbench.close(tool, tx);
    } else if tool == Tool::Search {
        project::open_search(state, tx);
    } else {
        state.workbench.open(tool);
        if tool == Tool::Outline {
            language::request_outline(state);
        }
        tx.instant();
    }
}

fn tools(state: &State, dock: Dock) -> Vec<Tool> {
    Tool::ALL
        .into_iter()
        .filter(|&tool| state.workbench.layout.side(tool, state.ui) == Some(dock))
        .filter(|&tool| tool != Tool::Preview || preview::available(state))
        .collect()
}

fn shown(state: &State, tool: Tool) -> bool {
    if tool == Tool::Preview {
        state.preview.visible
    } else {
        state.workbench.is_open(tool)
    }
}

pub(super) fn body(state: &State, center: El, palette: Palette, scale: f32) -> El {
    let left = tools(state, Dock::Left);
    let right = tools(state, Dock::Right);
    let bottom = tools(state, Dock::Bottom);
    let left_open = left.iter().any(|&tool| shown(state, tool));
    let right_open = right.iter().any(|&tool| shown(state, tool));
    let bottom_open = bottom.iter().any(|&tool| shown(state, tool));
    let mut middle = row()
        .key("workbench-middle")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0);
    middle = middle.child(strip(state, &left, Dock::Left, palette, scale));
    if left_open {
        middle = middle
            .child(area(state, &left, Dock::Left, right_open, palette, scale))
            .child(outer_divider(state, Dock::Left, palette, scale));
    }
    middle = middle.child(center);
    if right_open {
        middle = middle
            .child(outer_divider(state, Dock::Right, palette, scale))
            .child(area(state, &right, Dock::Right, left_open, palette, scale));
    }
    middle = middle.child(strip(state, &right, Dock::Right, palette, scale));
    let mut body = col()
        .key("workbench")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(middle)
        .on_layout_tx(|state: &mut State, [_, _, width, height], tx| {
            let layout = &mut state.workbench.layout;
            if layout.viewport == [width, height] {
                tx.live_only();
            } else {
                layout.viewport = [width, height];
                tx.instant();
            }
        });
    if bottom_open {
        body = body
            .child(outer_divider(state, Dock::Bottom, palette, scale))
            .child(area(state, &bottom, Dock::Bottom, false, palette, scale));
    }
    body.child(strip(state, &bottom, Dock::Bottom, palette, scale))
}

fn strip(state: &State, tools: &[Tool], dock: Dock, palette: Palette, scale: f32) -> El {
    let height = row_height(state.ui.density);
    let thickness = scale.recip();
    let name = format!("{}-strip", dock.key());
    let mut strip = if dock == Dock::Bottom {
        row()
            .h(height)
            .w_full()
            .center_y()
            .px(2.0)
            .border_sides([thickness, 0.0, 0.0, 0.0], palette.divider)
    } else {
        let border = if dock == Dock::Left {
            [0.0, thickness, 0.0, 0.0]
        } else {
            [0.0, 0.0, 0.0, thickness]
        };
        col()
            .w(height + 2.0)
            .h_full()
            .items_center()
            .pt(2.0)
            .border_sides(border, palette.divider)
    }
    .key(name)
    .fill(palette.chrome);
    if tools.is_empty() {
        return strip;
    }
    for &tool in tools {
        let on = shown(state, tool);
        let label = if dock == Dock::Bottom {
            tool.label()
        } else {
            tool.icon()
        };
        let mut button = item(
            palette,
            format!("dock-{}", tool.key()),
            label,
            height - 2.0,
            on,
            move |state, tx| toggle(state, tool, tx),
        )
        .label(tool.label())
        .role("switch")
        .checked(on)
        .help(tool.help());
        if dock != Dock::Bottom {
            button = button.w(height).min_w(height).px(0.0).justify_center();
        } else {
            button = button.w(16.0 + 7.0 * label.len() as f32);
        }
        strip = strip.child(button);
    }
    strip
}

fn area(
    state: &State,
    tools: &[Tool],
    dock: Dock,
    other: bool,
    palette: Palette,
    scale: f32,
) -> El {
    let extent = state.workbench.layout.extent(dock, other);
    let bottom = dock == Dock::Bottom;
    let mut group = if bottom {
        row().h(extent).w_full()
    } else {
        col().w(extent).h_full()
    }
    .key(format!("dock-area-{}", dock.key()))
    .min_h(0.0)
    .min_w(0.0)
    .clip_rounded(0.0)
    .fill(palette.chrome);
    let mut prior = None;
    for &tool in tools.iter().filter(|&&tool| shown(state, tool)) {
        if let Some(previous) = prior {
            group = group.child(inner_divider(previous, tool, bottom, palette, scale));
        }
        prior = Some(tool);
        let panel = if tool == Tool::Preview {
            preview::view::pane(state, palette, scale.recip(), scale)
        } else {
            tool_panel(state, tool, palette, scale)
        };
        let weight = state.workbench.layout.weights[tool as usize];
        let slot = if bottom {
            col().h_full()
        } else {
            col().w_full()
        };
        group = group.child(
            slot.key(format!("dock-panel-{}", tool.key()))
                .grow(weight)
                .min_h(0.0)
                .min_w(0.0)
                .clip_rounded(0.0)
                .child(panel),
        );
    }
    group
}
