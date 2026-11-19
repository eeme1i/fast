use std::time::Duration;

use fast_theme::tok::Palette;
use rune::{Color, El, Phase, Tx, col, el, row, text};

use crate::{State, view::action};

use super::{Camera, camera::Fit, toggle};

pub(crate) fn pane(state: &State, palette: Palette, hairline: f32, device_scale: f32) -> El {
    let preview = &state.preview;
    let size = state.ui.font_size;
    let camera = &preview.camera;
    let mut panel = col()
        .key("preview-pane")
        .role("region")
        .label("Document preview")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .fill(palette.canvas)
        .border_sides([0.0, 0.0, 0.0, hairline], palette.divider)
        .child(
            row()
                .key("preview-toolbar")
                .h(26.0)
                .px(4.0)
                .gap(2.0)
                .center_y()
                .fill(palette.chrome)
                .border_sides([0.0, 0.0, hairline, 0.0], palette.divider)
                .child(
                    action(
                        "preview-zoom-out",
                        "−",
                        palette,
                        size,
                        true,
                        |state, tx| zoom_center(state, 1.0 / 1.2, tx),
                    )
                    .help("Zoom out"),
                )
                .child(
                    text("")
                        .key("preview-zoom")
                        .live_text(&camera.zoom_label)
                        .w(39.0)
                        .font(palette.control_text(12.0))
                        .color(palette.text),
                )
                .child(
                    action("preview-zoom-in", "+", palette, size, true, |state, tx| {
                        zoom_center(state, 1.2, tx)
                    })
                    .help("Zoom in"),
                )
                .child(
                    text("")
                        .key("preview-page-number")
                        .live_text(&camera.page_label)
                        .font(palette.ui_text(12.0))
                        .color(palette.muted_text)
                        .truncate_fade()
                        .grow(1.0)
                        .min_w(0.0)
                        .ml(4.0),
                )
                .child(action(
                    "preview-view-options",
                    "View",
                    palette,
                    size,
                    true,
                    |state, tx| {
                        state.preview.settings_menu = !state.preview.settings_menu;
                        tx.instant();
                    },
                ))
                .child(
                    action("preview-close", "×", palette, size, true, toggle).help("Close preview"),
                ),
        );
    if preview.settings_menu {
        panel = panel.child(options(state, palette, hairline));
    }
    panel = panel.child(canvas(state, palette, device_scale));
    if let Some(message) = preview.error.as_deref().or(preview.message.as_deref()) {
        panel = panel.child(
            text(message)
                .key("preview-status")
                .h(22.0)
                .min_h(22.0)
                .px(6.0)
                .center_y()
                .truncate()
                .font(palette.ui_text(11.0))
                .color(if preview.error.is_some() {
                    palette.error_text
                } else {
                    palette.text
                })
                .fill(palette.chrome)
                .border_sides([hairline, 0.0, 0.0, 0.0], palette.divider)
                .help(message),
        );
    }
    panel
}

fn options(state: &State, palette: Palette, hairline: f32) -> El {
    let size = state.ui.font_size;
    col()
        .key("preview-options")
        .fill(palette.chrome)
        .px(4.0)
        .border_sides([0.0, 0.0, hairline, 0.0], palette.divider)
        .child(
            row()
                .gap(2.0)
                .child(action(
                    "preview-fit-width",
                    "Fit width",
                    palette,
                    size,
                    true,
                    |state, tx| fit(state, Fit::Width, tx),
                ))
                .child(action(
                    "preview-fit-page",
                    "Fit page",
                    palette,
                    size,
                    true,
                    |state, tx| fit(state, Fit::Page, tx),
                ))
                .child(action(
                    "preview-actual-size",
                    "100%",
                    palette,
                    size,
                    true,
                    |state, tx| {
                        let camera = &mut state.preview.camera;
                        camera.zoom_at(1.0, center(camera));
                        state.preview.settings_menu = false;
                        tx.instant();
                    },
                )),
        )
        .child(
            row()
                .gap(2.0)
                .child(action(
                    "preview-spread",
                    if state.preview.camera.spread {
                        "Single"
                    } else {
                        "Spread"
                    },
                    palette,
                    size,
                    true,
                    |state, tx| {
                        let preview = &mut state.preview;
                        let page = preview.camera.current_page();
                        preview.camera.spread = !preview.camera.spread;
                        preview.camera.layout(&preview.pages);
                        preview.camera.reveal(fast_preview::PagePoint {
                            page,
                            x: 0.0,
                            y: 0.0,
                        });
                        preview.settings_menu = false;
                        tx.instant();
                    },
                ))
                .child(
                    action(
                        "preview-invert",
                        if state.preview.inverted {
                            "Light"
                        } else {
                            "Dark"
                        },
                        palette,
                        size,
                        true,
                        |state, tx| {
                            state.preview.inverted = !state.preview.inverted;
                            state.preview.settings_menu = false;
                            tx.instant();
                        },
                    )
                    .help("Toggle dark paper"),
                )
                .child(
                    action(
                        "preview-follow",
                        if state.preview.follow {
                            "Unfollow"
                        } else {
                            "Follow"
                        },
                        palette,
                        size,
                        true,
                        |state, tx| {
                            state.preview.follow = !state.preview.follow;
                            state.preview.caret = None;
                            state.preview.settings_menu = false;
                            tx.instant();
                        },
                    )
                    .help("Follow the editor caret"),
                ),
        )
}

fn canvas(state: &State, palette: Palette, device_scale: f32) -> El {
    let preview = &state.preview;
    let camera = &preview.camera;
    let mut world = el()
        .key("preview-world")
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        // Keep the camera's painted geometry the same during gestures and at rest.
        .pixel_snap(false)
        .origin(0.0, 0.0)
        .live_offset(&camera.offset)
        .live_scale(&camera.zoom)
        .inert();
    for image in &preview.images {
        if let Some(sheet) = camera.sheets.get(image.page) {
            world = world.child(
                el().key(format!("preview-page-{}", image.page + 1))
                    .absolute(sheet.x, sheet.y)
                    .size(sheet.size.width, sheet.size.height)
                    .fill(if preview.inverted {
                        rune::oklch(0.206, 0.0, 0.0)
                    } else {
                        Color::WHITE
                    })
                    .pixels(&image.pixels),
            );
        }
    }
    el().key("preview-canvas")
        .role("group")
        .label("Rendered pages. Double-click to jump to source.")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(palette.chrome)
        .clip_rounded(0.0)
        .child(world)
        .on_layout_tx(move |state: &mut State, [_, _, w, h], tx| {
            let camera = &mut state.preview.camera;
            camera.viewport = [w, h];
            camera.device_scale = device_scale;
            camera.apply_fit();
            wake_live(tx);
        })
        .on_wheel(|state: &mut State, event, tx| {
            let camera = &mut state.preview.camera;
            if event.mods.ctrl || event.mods.cmd {
                camera.zoom_at(
                    camera.zoom.get() * (event.dy * 0.009).exp(),
                    [event.x, event.y],
                );
                state.preview.zooming = true;
            } else {
                let [x, y] = camera.offset.get();
                camera.pan([x + event.dx, y + event.dy]);
            }
            wake_live(tx);
            true
        })
        .on_magnify(|state: &mut State, event, tx| {
            let camera = &mut state.preview.camera;
            camera.zoom_at(camera.zoom.get() * (1.0 + event.delta), [event.x, event.y]);
            state.preview.zooming = true;
            wake_live(tx);
            true
        })
        .on_drag(|state: &mut State, event, tx| {
            let preview = &mut state.preview;
            match event.phase {
                Phase::Down => preview.drag_start = preview.camera.offset.get(),
                Phase::Move => {
                    let [x, y] = preview.drag_start;
                    preview.camera.pan([x + event.dx, y + event.dy]);
                }
                Phase::Up
                    if !event.cancelled()
                        && event.clicks == 2
                        && event.dx.abs().max(event.dy.abs()) <= 4.0 =>
                {
                    if let (Some(generation), Some(worker), Some(point)) = (
                        preview.ready,
                        &preview.worker,
                        preview.camera.hit(event.x, event.y),
                    ) {
                        worker.source(generation, point);
                    }
                }
                Phase::Up => {}
            }
            wake_live(tx);
        })
}

fn center(camera: &Camera) -> [f32; 2] {
    [camera.viewport[0] / 2.0, camera.viewport[1] / 2.0]
}

fn zoom_center(state: &mut State, factor: f32, tx: &mut Tx) {
    let camera = &mut state.preview.camera;
    camera.zoom_at(camera.zoom.get() * factor, center(camera));
    state.preview.zooming = true;
    wake_live(tx);
}

fn fit(state: &mut State, mode: Fit, tx: &mut Tx) {
    state.preview.camera.fit = mode;
    state.preview.camera.apply_fit();
    state.preview.settings_menu = false;
    tx.instant();
}

fn wake_live(tx: &mut Tx) {
    tx.wake_after(Duration::ZERO);
    tx.live_only();
}
