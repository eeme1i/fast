use fast_theme::tok::Palette;
use rune::{Cursor, El, Phase, el, row};

use super::{Dock, Tool, shown, tools};
use crate::State;

pub(super) fn outer_divider(state: &State, dock: Dock, palette: Palette, scale: f32) -> El {
    let horizontal = dock == Dock::Bottom;
    let axis = usize::from(horizontal);
    let sign = if dock == Dock::Left { 1.0 } else { -1.0 };
    let index = dock as usize;
    let viewport = state.workbench.layout.viewport[axis].max(1.0);
    let other = match dock {
        Dock::Left => Dock::Right,
        Dock::Right => Dock::Left,
        Dock::Bottom => Dock::Bottom,
    };
    let other_open = !horizontal && tools(state, other).iter().any(|&tool| shown(state, tool));
    let bounds: (f32, f32) = if horizontal {
        (0.18, 0.68)
    } else if other_open {
        (0.17, (viewport - 56.0).max(1.0) * 0.34 / viewport)
    } else {
        (0.17, (viewport - 56.0).max(1.0) * 0.62 / viewport)
    };
    divider(format!("resize-{}", dock.key()), horizontal, palette, scale).on_drag(
        move |state: &mut State, drag, tx| {
            let layout = &mut state.workbench.layout;
            match drag.phase {
                Phase::Down => layout.drag[0] = layout.extent(dock, other_open) / viewport,
                Phase::Move => {
                    let travel = if horizontal { drag.dy } else { drag.dx };
                    layout.ratios[index] = (layout.drag[0] + sign * travel / viewport)
                        .clamp(bounds.0.min(bounds.1), bounds.1);
                    tx.instant();
                }
                Phase::Up if drag.cancelled() => {
                    layout.ratios[index] = layout.drag[0];
                    tx.instant();
                }
                Phase::Up => {
                    state.workbench.save_layout();
                    tx.live_only();
                }
            }
        },
    )
}

pub(super) fn inner_divider(
    first: Tool,
    second: Tool,
    bottom: bool,
    palette: Palette,
    scale: f32,
) -> El {
    let a = first as usize;
    let b = second as usize;
    divider(
        format!("split-{}-{}", first.key(), second.key()),
        !bottom,
        palette,
        scale,
    )
    .on_drag(move |state: &mut State, drag, tx| {
        let layout = &mut state.workbench.layout;
        match drag.phase {
            Phase::Down => layout.drag = [layout.weights[a], layout.weights[b]],
            Phase::Move => {
                let total = layout.drag[0] + layout.drag[1];
                let (travel, dimension) = if bottom {
                    (drag.dx, layout.viewport[0])
                } else {
                    (drag.dy, layout.viewport[1])
                };
                let first = (layout.drag[0] + travel * 2.0 / dimension.max(1.0))
                    .clamp(total * 0.18, total * 0.82);
                layout.weights[a] = first;
                layout.weights[b] = total - first;
                tx.instant();
            }
            Phase::Up if drag.cancelled() => {
                [layout.weights[a], layout.weights[b]] = layout.drag;
                tx.instant();
            }
            Phase::Up => {
                state.workbench.save_layout();
                tx.live_only();
            }
        }
    })
}

fn divider(key: String, horizontal: bool, palette: Palette, scale: f32) -> El {
    let line = if horizontal {
        el().w_full().h(scale.recip())
    } else {
        el().h_full().w(scale.recip())
    };
    let hit = if horizontal {
        row().h(4.0).w_full().cursor(Cursor::ResizeRow)
    } else {
        row().w(4.0).h_full().cursor(Cursor::ResizeColumn)
    };
    hit.key(key)
        .role("separator")
        .center()
        .fill(palette.chrome)
        .hover(palette.hover)
        .child(line.fill(palette.divider).inert().a11y_hidden())
}
