use fast_core::config::{Density, FontFamily, Theme};
use rune::tok::{Family, TextStyle};
use rune::{Color, oklch};

pub const HEADER_HEIGHT: f32 = 28.0;
pub const STATUS_HEIGHT: f32 = 24.0;

pub fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 22.0,
        Density::Standard => 24.0,
        Density::Comfortable => 28.0,
    }
}

pub fn ui_text(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0).lh(size + 4.0)
}

pub fn control_text(size: f32) -> TextStyle {
    TextStyle::new(size, 500.0).lh(size + 4.0)
}

#[derive(Clone, Copy)]
pub struct Palette {
    font: Family,
    pub canvas: Color,
    pub chrome: Color,
    pub divider: Color,
    pub text: Color,
    pub muted_text: Color,
    pub error_text: Color,
    pub selected: Color,
    pub selected_hover: Color,
    pub hover: Color,
    pub accent: Color,
}

impl Palette {
    pub fn with_font(mut self, font: FontFamily) -> Self {
        self.font = match font {
            FontFamily::Inter => Family::Sans,
            FontFamily::SystemMono => Family::SystemMono,
            FontFamily::Serif => Family::Named("Georgia, Times New Roman, Liberation Serif"),
        };
        self
    }

    pub fn ui_text(self, size: f32) -> TextStyle {
        ui_text(size).family(self.font)
    }

    pub fn control_text(self, size: f32) -> TextStyle {
        control_text(size).family(self.font)
    }
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::DarculaClassic => Palette {
            font: Family::Sans,
            canvas: oklch(0.284, 0.004, 260.0),
            chrome: oklch(0.337, 0.006, 260.0),
            divider: oklch(0.232, 0.006, 260.0),
            text: oklch(0.875, 0.008, 260.0),
            muted_text: oklch(0.730, 0.015, 260.0),
            error_text: oklch(0.781, 0.120, 30.0),
            selected: oklch(0.416, 0.036, 250.0),
            selected_hover: oklch(0.470, 0.064, 250.0),
            hover: oklch(0.398, 0.012, 260.0),
            accent: oklch(0.742, 0.116, 249.0),
        },
        Theme::IntelliJLightClassic => Palette {
            font: Family::Sans,
            canvas: oklch(0.989, 0.002, 260.0),
            chrome: oklch(0.928, 0.005, 260.0),
            divider: oklch(0.791, 0.008, 260.0),
            text: oklch(0.287, 0.014, 260.0),
            muted_text: oklch(0.472, 0.018, 260.0),
            error_text: oklch(0.439, 0.147, 28.0),
            selected: oklch(0.876, 0.039, 249.0),
            selected_hover: oklch(0.827, 0.066, 249.0),
            hover: oklch(0.894, 0.011, 260.0),
            accent: oklch(0.504, 0.153, 252.0),
        },
    }
}

#[derive(Clone, Copy)]
pub struct CodePalette {
    pub active_line: Color,
    pub selection: Color,
    pub search: Color,
    pub comment: Color,
    pub keyword: Color,
    pub string: Color,
    pub code: Color,
    pub markup: Color,
    pub punctuation: Color,
    pub error: Color,
}

pub fn code_palette(theme: Theme) -> CodePalette {
    match theme {
        Theme::DarculaClassic => CodePalette {
            active_line: oklch(0.322, 0.009, 258.0),
            selection: oklch(0.445, 0.074, 251.0),
            search: oklch(0.435, 0.067, 83.0),
            comment: oklch(0.700, 0.028, 145.0),
            keyword: oklch(0.780, 0.120, 63.0),
            string: oklch(0.766, 0.091, 144.0),
            code: oklch(0.778, 0.089, 237.0),
            markup: oklch(0.800, 0.079, 298.0),
            punctuation: oklch(0.755, 0.021, 255.0),
            error: oklch(0.788, 0.114, 30.0),
        },
        Theme::IntelliJLightClassic => CodePalette {
            active_line: oklch(0.952, 0.012, 248.0),
            selection: oklch(0.865, 0.064, 242.0),
            search: oklch(0.889, 0.100, 92.0),
            comment: oklch(0.482, 0.041, 145.0),
            keyword: oklch(0.443, 0.121, 44.0),
            string: oklch(0.444, 0.102, 145.0),
            code: oklch(0.442, 0.105, 250.0),
            markup: oklch(0.473, 0.122, 301.0),
            punctuation: oklch(0.464, 0.021, 255.0),
            error: oklch(0.439, 0.147, 28.0),
        },
    }
}
