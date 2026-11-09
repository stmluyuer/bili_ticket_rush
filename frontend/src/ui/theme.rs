use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub sunken: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub text_3: Color32,
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_soft: Color32,
    pub on_primary: Color32,
    pub info: Color32,
    pub info_soft: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub neutral: Color32,
    pub neutral_soft: Color32,
    pub scrim: Color32,
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        let c = if dark {
            [0x151418, 0x1B1A1F, 0x222026, 0x1A191D, 0x33303A, 0x46424F,
             0xECEAF0, 0xB9B4C1, 0x8E8999, 0xF07BA4, 0xF596B8, 0x3A1F2A,
             0x2A0F1A, 0x7FAEFF, 0x1E2A40, 0x5FCB8A, 0x183326, 0xF0B455,
             0x3A2C14, 0xFF7F86, 0x3D1C20, 0xA39FAB, 0x2A2830]
        } else {
            [0xF6F5F7, 0xEEEDF1, 0xFFFFFF, 0xF1F0F3, 0xE3E1E7, 0xCFCCD5,
             0x1E1B22, 0x57525E, 0x74707C, 0xC8336A, 0xAE2A5B, 0xFBE8EF,
             0xFFFFFF, 0x2F6FDB, 0xE7EFFC, 0x1B7F45, 0xE4F4EA, 0xA86200,
             0xFFF2DD, 0xCC2F3C, 0xFCE7E8, 0x6B6673, 0xEFEDF2]
        };
        Self {
            bg: rgb(c[0]), sidebar: rgb(c[1]), surface: rgb(c[2]), sunken: rgb(c[3]),
            border: rgb(c[4]), border_strong: rgb(c[5]), text: rgb(c[6]), text_2: rgb(c[7]),
            text_3: rgb(c[8]), primary: rgb(c[9]), primary_hover: rgb(c[10]),
            primary_soft: rgb(c[11]), on_primary: rgb(c[12]), info: rgb(c[13]),
            info_soft: rgb(c[14]), success: rgb(c[15]), success_soft: rgb(c[16]),
            warning: rgb(c[17]), warning_soft: rgb(c[18]), danger: rgb(c[19]),
            danger_soft: rgb(c[20]), neutral: rgb(c[21]), neutral_soft: rgb(c[22]),
            scrim: Color32::from_black_alpha(if dark { 153 } else { 102 }),
        }
    }
}

/// 状态语气：标签、消息条、提示块共用
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone { Neutral, Primary, Info, Success, Warning, Danger }

impl Palette {
    /// 返回 (主色, 浅底色)
    pub fn tone(&self, tone: Tone) -> (Color32, Color32) {
        match tone {
            Tone::Neutral => (self.neutral, self.neutral_soft),
            Tone::Primary => (self.primary, self.primary_soft),
            Tone::Info => (self.info, self.info_soft),
            Tone::Success => (self.success, self.success_soft),
            Tone::Warning => (self.warning, self.warning_soft),
            Tone::Danger => (self.danger, self.danger_soft),
        }
    }
}

pub fn palette(ctx: &egui::Context) -> Palette {
    Palette::new(ctx.style().visuals.dark_mode)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn apply_theme(ctx: &egui::Context, dark: bool, scale: f32, native_ppp: f32) {
    let p = Palette::new(dark);
    let mut style = (*ctx.style()).clone();
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.surface;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.extreme_bg_color = p.sunken;
    visuals.faint_bg_color = p.sunken;
    visuals.code_bg_color = p.sunken;
    visuals.hyperlink_color = p.primary;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.danger;
    visuals.selection.bg_fill = p.primary_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.primary);
    visuals.window_rounding = 12.0.into();
    visuals.window_shadow = if dark { egui::epaint::Shadow::small_dark() } else { egui::epaint::Shadow::small_light() };
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.text_cursor = Stroke::new(1.5, p.primary);
    for w in [&mut visuals.widgets.noninteractive, &mut visuals.widgets.inactive,
              &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
        w.bg_fill = p.surface;
        w.weak_bg_fill = p.surface;
        w.bg_stroke = Stroke::new(1.0, p.border);
        w.fg_stroke = Stroke::new(1.0, p.text);
        w.rounding = 6.0.into();
        w.expansion = 0.0;
    }
    visuals.widgets.noninteractive.fg_stroke.color = p.text_2;
    visuals.widgets.inactive.bg_stroke.color = p.border_strong;
    visuals.widgets.hovered.bg_fill = p.sunken;
    visuals.widgets.hovered.weak_bg_fill = p.sunken;
    visuals.widgets.hovered.bg_stroke.color = p.text_3;
    visuals.widgets.active.bg_fill = p.sunken;
    visuals.widgets.active.weak_bg_fill = p.sunken;
    visuals.widgets.active.bg_stroke.color = p.primary;
    visuals.widgets.open.bg_fill = p.sunken;
    visuals.widgets.open.weak_bg_fill = p.sunken;
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Heading, bold(20.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
        (TextStyle::Name("section".into()), bold(16.0)),
        (TextStyle::Name("metric".into()), FontId::monospace(24.0)),
        (TextStyle::Name("display".into()), FontId::monospace(28.0)),
    ].into_iter().collect();
    ctx.set_style(style);
    ctx.set_pixels_per_point(native_ppp * scale);
}

pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("chinese_font".into(), egui::FontData::from_owned(load_system_font()));
    for family in fonts.families.values_mut() {
        family.insert(0, "chinese_font".into());
    }
    let mut bold_fonts = fonts.families[&FontFamily::Proportional].clone();
    #[cfg(target_os = "windows")]
    if let Ok(data) = std::fs::read("C:/Windows/Fonts/msyhbd.ttc") {
        fonts.font_data.insert("chinese_bold".into(), egui::FontData::from_owned(data));
        bold_fonts.insert(0, "chinese_bold".into());
    }
    fonts.families.insert(FontFamily::Name("bold".into()), bold_fonts);
    ctx.set_fonts(fonts);
}

fn load_system_font() -> Vec<u8> {
    #[cfg(target_os = "windows")]
    let paths = ["C:/Windows/Fonts/msyh.ttc", "C:/Windows/Fonts/simhei.ttf", "C:/Windows/Fonts/simsun.ttc", "C:/Windows/Fonts/msyh.ttf"];
    #[cfg(target_os = "macos")]
    let paths = ["/System/Library/Fonts/PingFang.ttc", "/Library/Fonts/Arial Unicode.ttf", "/System/Library/Fonts/STHeiti Light.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"];
    #[cfg(target_os = "linux")]
    let paths = ["/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf", "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"];
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    for path in paths {
        if let Ok(data) = std::fs::read(path) {
            log::info!("加载字体: {}", path);
            return data;
        }
    }
    log::warn!("无法加载系统中文字体，使用内置字体");
    include_bytes!("../../../resources/fonts/NotoSansSC-Regular.otf").to_vec()
}
