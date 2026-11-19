use std::ops::Range;

use fast_preview::{PagePoint, PageSize, RasterRequest};
use rune::{Live, LiveText};

const EDGE: f32 = 12.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Fit {
    #[default]
    Width,
    Page,
    Custom,
}

pub(super) struct Camera {
    pub offset: Live<[f32; 2]>,
    pub zoom: Live<f32>,
    pub zoom_label: LiveText,
    pub page_label: LiveText,
    pub viewport: [f32; 2],
    pub device_scale: f32,
    pub fit: Fit,
    pub spread: bool,
    pub sheets: Vec<Sheet>,
    size: [f32; 2],
}

#[derive(Clone, Copy)]
pub(super) struct Sheet {
    pub x: f32,
    pub y: f32,
    pub size: PageSize,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: Live::new([0.0, EDGE]),
            zoom: Live::new(1.0),
            zoom_label: LiveText::new("100%"),
            page_label: LiveText::new("– / –"),
            viewport: [0.0; 2],
            device_scale: 1.0,
            fit: Fit::Width,
            spread: false,
            sheets: Vec::new(),
            size: [0.0; 2],
        }
    }
}

impl Camera {
    pub fn layout(&mut self, pages: &[PageSize]) {
        self.sheets.clear();
        let columns = if self.spread { 2 } else { 1 };
        let mut y = 0.0_f32;
        let mut max_width = 0.0_f32;
        for row in pages.chunks(columns) {
            let mut x = 0.0_f32;
            let mut height = 0.0_f32;
            for &size in row {
                self.sheets.push(Sheet { x, y, size });
                x += size.width + EDGE;
                height = height.max(size.height);
            }
            max_width = max_width.max(x - EDGE);
            y += height + EDGE;
        }
        for row in self.sheets.chunks_mut(columns) {
            if let Some(last) = row.last() {
                let inset = (max_width - last.x - last.size.width) / 2.0;
                for sheet in row {
                    sheet.x += inset;
                }
            }
        }
        self.size = [max_width, (y - EDGE).max(0.0)];
        self.apply_fit();
    }

    pub fn apply_fit(&mut self) {
        let [width, height] = self.viewport;
        let max_height = self.sheets.first().map_or(0.0, |sheet| {
            if self.spread {
                self.sheets.get(1).map_or(sheet.size.height, |other| {
                    sheet.size.height.max(other.size.height)
                })
            } else {
                sheet.size.height
            }
        });
        let old_zoom = self.zoom.get();
        let new_zoom = match self.fit {
            Fit::Custom => old_zoom,
            _ if self.size[0] == 0.0 || width <= 0.0 || height <= 0.0 => old_zoom,
            Fit::Width => (width - 2.0 * EDGE) / self.size[0],
            Fit::Page => ((width - 2.0 * EDGE) / self.size[0])
                .min((height - 2.0 * EDGE) / max_height.max(1.0)),
        }
        .clamp(0.05, 8.0);
        let old_offset = self.offset.get();
        let ratio = new_zoom / old_zoom;
        self.set(new_zoom, [old_offset[0], old_offset[1] * ratio]);
    }

    pub fn zoom_at(&mut self, zoom: f32, point: [f32; 2]) {
        self.fit = Fit::Custom;
        let zoom = zoom.clamp(0.05, 8.0);
        let ratio = zoom / self.zoom.get();
        let [x, y] = self.offset.get();
        self.set(
            zoom,
            [
                point[0] - (point[0] - x) * ratio,
                point[1] - (point[1] - y) * ratio,
            ],
        );
    }

    pub fn pan(&mut self, offset: [f32; 2]) {
        self.set(self.zoom.get(), offset);
    }

    fn set(&self, zoom: f32, offset: [f32; 2]) {
        let x = clamp_axis(offset[0], self.size[0] * zoom, self.viewport[0], true);
        let y = clamp_axis(offset[1], self.size[1] * zoom, self.viewport[1], false);
        self.offset.set([x, y]);
        self.zoom.set(zoom);
        let zoom_label = format!("{:.0}%", zoom * 100.0);
        if self.zoom_label.get().as_ref() != zoom_label {
            self.zoom_label.set(zoom_label);
        }
        let page_label = if self.sheets.is_empty() {
            "– / –".to_owned()
        } else {
            format!("{} / {}", self.current_page() + 1, self.sheets.len())
        };
        if self.page_label.get().as_ref() != page_label {
            self.page_label.set(page_label);
        }
    }

    pub fn visible(&self) -> Range<usize> {
        let zoom = self.zoom.get();
        let y = self.offset.get()[1];
        let top = -y / zoom;
        let bottom = (self.viewport[1] - y) / zoom;
        let start = self
            .sheets
            .partition_point(|sheet| sheet.y + self.row_height(sheet.y) < top);
        let end = self.sheets.partition_point(|sheet| sheet.y < bottom);
        let neighbors = if self.spread { 2 } else { 1 };
        start.saturating_sub(neighbors)..end.saturating_add(neighbors).min(self.sheets.len())
    }

    fn row_height(&self, y: f32) -> f32 {
        let index = self.sheets.partition_point(|sheet| sheet.y < y);
        self.sheets[index..]
            .iter()
            .take_while(|sheet| sheet.y == y)
            .map(|sheet| sheet.size.height)
            .fold(0.0, f32::max)
    }

    pub fn current_page(&self) -> usize {
        let center = (self.viewport[1] * 0.4 - self.offset.get()[1]) / self.zoom.get();
        self.sheets
            .partition_point(|sheet| sheet.y + self.row_height(sheet.y) < center)
            .min(self.sheets.len().saturating_sub(1))
    }

    pub fn raster(&self, inverted: bool) -> RasterRequest {
        RasterRequest {
            pages: self.visible(),
            scale: (self.zoom.get() * self.device_scale * 8.0)
                .ceil()
                .clamp(1.0, 64.0) as u16,
            inverted,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<PagePoint> {
        let [base_x, base_y] = self.offset.get();
        let zoom = self.zoom.get();
        let point = [(x - base_x) / zoom, (y - base_y) / zoom];
        self.visible().find_map(|page| {
            let sheet = self.sheets[page];
            let [x, y] = [point[0] - sheet.x, point[1] - sheet.y];
            (x >= 0.0 && y >= 0.0 && x < sheet.size.width && y < sheet.size.height)
                .then_some(PagePoint { page, x, y })
        })
    }

    pub fn reveal(&mut self, point: PagePoint) {
        let Some(sheet) = self.sheets.get(point.page) else {
            return;
        };
        let [base_x, base_y] = self.offset.get();
        let zoom = self.zoom.get();
        let y = base_y + (sheet.y + point.y) * zoom;
        if y < 20.0 || y > self.viewport[1] - 20.0 {
            self.pan([base_x, self.viewport[1] * 0.4 - (sheet.y + point.y) * zoom]);
        }
    }
}

fn clamp_axis(value: f32, content: f32, viewport: f32, centered: bool) -> f32 {
    if content + 2.0 * EDGE <= viewport {
        if centered {
            (viewport - content) / 2.0
        } else {
            EDGE
        }
    } else {
        value.clamp(viewport - content - EDGE, EDGE)
    }
}
