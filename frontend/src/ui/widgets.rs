use std::f32::consts::PI;

use eframe::egui::{
    self, epaint::PathShape, load::SizedTexture, pos2, vec2, Align, Align2, Color32, CursorIcon,
    FontId, Layout, Margin, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    TextureHandle, Ui,
};
use qrcode::QrCode;

use super::theme::{bold, palette, Palette, Tone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Primary, Secondary, Ghost, Danger, DangerGhost }

/// Sm 28 / Md 32 / Lg 36 / Xl 40（Xl 撑满可用宽度）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size { Sm, Md, Lg, Xl }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    None,
    /// ASCII 字母数字与 `@ . - _`
    Token,
    Digits,
}

/// 购票人卡片左侧的勾选样式；None 为只读展示
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check { None, Single, Multi }

#[derive(Default)]
pub struct FieldOpts<'a> {
    pub label: Option<&'a str>,
    pub hint: &'a str,
    pub error: Option<String>,
    pub password: bool,
    pub width: Option<f32>,
    /// 0 为单行
    pub multiline_rows: usize,
    pub filter: Filter,
    /// 输入框内左侧的固定文字，例如 “+86”
    pub prefix: Option<&'a str>,
    /// 稳定的输入框 ID；同一界面里输入框数量会变化时传入，避免焦点跳到别的输入框
    pub id: Option<&'a str>,
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    egui::Frame::none()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .rounding(10.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// 左侧节标题，右侧操作区按从右到左的顺序添加
pub fn section_title(ui: &mut Ui, text: &str, right: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).font(bold(16.0)).color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

/// 页面标题（20 粗体），右侧操作区按从右到左的顺序添加
pub fn page_title(ui: &mut Ui, text: &str, right: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.label(RichText::new(text).font(bold(20.0)).color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

pub fn small(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    ui.label(RichText::new(text.into()).size(12.0).color(color))
}

pub fn pill(ui: &mut Ui, text: impl Into<String>, tone: Tone) -> Response {
    let p = palette(ui.ctx());
    let (fg, bg) = p.tone(tone);
    let galley = ui.painter().layout_no_wrap(text.into(), FontId::proportional(12.0), fg);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 22.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 11.0, bg);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley);
    }
    resp
}

pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    button_ex(ui, text, kind, Size::Md, false, true)
}

pub fn button_ex(ui: &mut Ui, text: &str, kind: Kind, size: Size, busy: bool, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let (height, pad, font_size) = match size {
        Size::Sm => (28.0, 10.0, 13.0),
        Size::Md => (32.0, 14.0, 14.0),
        Size::Lg => (36.0, 16.0, 14.0),
        Size::Xl => (40.0, 18.0, 15.0),
    };
    let font = FontId::proportional(font_size);
    let interactive = enabled && !busy;
    let spinner_w = if busy { 20.0 } else { 0.0 };
    let text_w = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.text).size().x;
    let width = if size == Size::Xl { ui.available_width() } else { text_w + pad * 2.0 + spinner_w };
    let sense = if interactive { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), sense);

    if ui.is_rect_visible(rect) {
        let active = interactive && (resp.hovered() || resp.is_pointer_button_down_on());
        let none = Stroke::NONE;
        let (fill, stroke, fg) = match kind {
            Kind::Primary => (if active { p.primary_hover } else { p.primary }, none, p.on_primary),
            Kind::Secondary => (
                if active { p.sunken } else { p.surface },
                Stroke::new(1.0, if active { p.text_3 } else { p.border_strong }),
                p.text,
            ),
            Kind::Ghost => (
                if active { p.sunken } else { Color32::TRANSPARENT },
                none,
                if active { p.text } else { p.text_2 },
            ),
            Kind::Danger => (if active { p.danger.gamma_multiply(0.86) } else { p.danger }, none, p.on_primary),
            Kind::DangerGhost => (if active { p.danger_soft } else { Color32::TRANSPARENT }, none, p.danger),
        };
        let alpha = if enabled { 1.0 } else { 0.45 };
        let painter = ui.painter();
        painter.rect_filled(rect, 6.0, fill.gamma_multiply(alpha));
        if stroke.width > 0.0 {
            painter.rect_stroke(rect, 6.0, Stroke::new(stroke.width, stroke.color.gamma_multiply(alpha)));
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), 8.0, Stroke::new(2.0, p.primary));
        }
        let fg = fg.gamma_multiply(alpha);
        let mut x = rect.center().x - (text_w + spinner_w) / 2.0;
        if busy {
            let spin_rect = Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0));
            egui::Spinner::new().size(14.0).color(fg).paint_at(ui, spin_rect);
            x += spinner_w;
        }
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, font, fg);
    }
    if interactive { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
}

pub fn segmented<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    egui::Frame::none()
        .fill(p.sunken)
        .stroke(Stroke::new(1.0, p.border))
        .rounding(7.0)
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (option, label) in options {
                    let selected = *value == *option;
                    let font = FontId::proportional(13.0);
                    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text).size().x;
                    let (rect, resp) = ui.allocate_exact_size(vec2(w + 24.0, 26.0), Sense::click());
                    let hovered = resp.hovered();
                    if selected {
                        ui.painter().rect_filled(rect, 5.0, p.surface);
                        ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, p.border));
                    }
                    if resp.has_focus() {
                        ui.painter().rect_stroke(rect.expand(1.0), 6.0, Stroke::new(2.0, p.primary));
                    }
                    let color = if selected || hovered { p.text } else { p.text_2 };
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, *label, font, color);
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !selected {
                        *value = option.clone();
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// 带标签、提示、错误和过滤的输入框。返回内部 TextEdit 的响应（可判断回车、失焦）
pub fn text_field(ui: &mut Ui, value: &mut String, opts: FieldOpts) -> Response {
    let p = palette(ui.ctx());
    let width = opts.width.unwrap_or_else(|| ui.available_width());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.set_width(width);
        if let Some(label) = opts.label {
            small(ui, label, p.text_2);
        }
        // 背景在输入框之后才能确定描边颜色（获焦 / 出错），先占位
        let bg = ui.painter().add(Shape::Noop);
        let frame = egui::Frame::none().inner_margin(Margin::symmetric(10.0, 8.0)).show(ui, |ui| {
            ui.set_width(width - 20.0);
            let layout = if opts.multiline_rows > 0 { Layout::left_to_right(Align::Min) } else { Layout::left_to_right(Align::Center) };
            ui.with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(prefix) = opts.prefix {
                    ui.label(RichText::new(prefix).color(p.text_2));
                }
                let edit_id = match opts.id {
                    Some(id) => ui.make_persistent_id(id),
                    None => ui.next_auto_id(),
                };
                let show_id = edit_id.with("show");
                let shown = ui.data(|d| d.get_temp::<bool>(show_id)).unwrap_or(false);
                let toggle_w = if opts.password { 40.0 } else { 0.0 };
                let edit = if opts.multiline_rows > 0 {
                    egui::TextEdit::multiline(value).desired_rows(opts.multiline_rows)
                } else {
                    egui::TextEdit::singleline(value)
                };
                let resp = ui.add(
                    edit.id(edit_id)
                        .frame(false)
                        .margin(egui::Vec2::ZERO)
                        .hint_text(RichText::new(opts.hint).color(p.text_3))
                        .password(opts.password && !shown)
                        .text_color(p.text)
                        .desired_width((ui.available_width() - toggle_w).max(40.0)),
                );
                if opts.password {
                    let label = if shown { "隐藏" } else { "显示" };
                    let toggle = ui.add(egui::Label::new(RichText::new(label).size(12.0).color(p.primary)).sense(Sense::click()));
                    if toggle.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        ui.data_mut(|d| d.insert_temp(show_id, !shown));
                    }
                }
                resp
            })
            .inner
        });
        let resp = frame.inner;
        if resp.changed() {
            match opts.filter {
                Filter::None => {}
                Filter::Token => value.retain(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_')),
                Filter::Digits => value.retain(|c| c.is_ascii_digit()),
            }
        }
        let stroke = if opts.error.is_some() {
            p.danger
        } else if resp.has_focus() {
            p.primary
        } else {
            p.border_strong
        };
        let rect = frame.response.rect;
        ui.painter().set(bg, Shape::Vec(vec![
            Shape::rect_filled(rect, 6.0, p.sunken),
            Shape::rect_stroke(rect, 6.0, Stroke::new(1.0, stroke)),
        ]));
        if let Some(error) = &opts.error {
            small(ui, error.as_str(), p.danger);
        }
        resp
    })
    .inner
}

pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = palette(ui.ctx());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    if ui.is_rect_visible(rect) {
        let alpha = if ui.is_enabled() { 1.0 } else { 0.45 };
        let track = mix(p.border_strong, p.primary, t).gamma_multiply(alpha);
        ui.painter().rect_filled(rect, 11.0, track);
        let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
        ui.painter().circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE.gamma_multiply(alpha));
        if resp.has_focus() {
            ui.painter().rect_stroke(rect.expand(2.0), 13.0, Stroke::new(2.0, p.primary));
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn stepper(ui: &mut Ui, value: &mut i32, min: i32, max: i32) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if button_ex(ui, "−", Kind::Secondary, Size::Sm, false, *value > min).clicked() {
            *value -= 1;
            changed = true;
        }
        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 28.0), Sense::hover());
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, value.to_string(), FontId::monospace(14.0), p.text);
        if button_ex(ui, "+", Kind::Secondary, Size::Sm, false, *value < max).clicked() {
            *value += 1;
            changed = true;
        }
    });
    changed
}

/// 购票人选择卡。选中：primary 描边 + primary_soft 底；左侧画单选圈或复选框
pub fn select_card(ui: &mut Ui, selected: bool, check: Check, width: f32, add: impl FnOnce(&mut Ui)) -> Response {
    let p = palette(ui.ctx());
    let bg = ui.painter().add(Shape::Noop);
    let mut mark_rect = None;
    let inner = egui::Frame::none().inner_margin(12.0).show(ui, |ui| {
        ui.set_width(width - 24.0);
        ui.horizontal_top(|ui| {
            if check != Check::None {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                mark_rect = Some(r);
            }
            ui.vertical(add);
        });
    });
    let rect = inner.response.rect;
    let resp = if check == Check::None { inner.response } else { inner.response.interact(Sense::click()) };
    let hovered = check != Check::None && resp.hovered();
    let (fill, stroke) = if selected {
        (p.primary_soft, p.primary)
    } else if hovered {
        (p.surface, p.border_strong)
    } else {
        (p.surface, p.border)
    };
    let mut shapes = vec![Shape::rect_filled(rect, 8.0, fill), Shape::rect_stroke(rect, 8.0, Stroke::new(1.0, stroke))];
    if resp.has_focus() {
        shapes.push(Shape::rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0, p.primary)));
    }
    if let Some(r) = mark_rect {
        match check {
            Check::Multi => {
                let r = r.shrink(1.0);
                if selected {
                    shapes.push(Shape::rect_filled(r, 4.0, p.primary));
                    let pts = vec![
                        pos2(r.left() + 3.5, r.center().y),
                        pos2(r.left() + 6.5, r.bottom() - 4.0),
                        pos2(r.right() - 3.0, r.top() + 4.0),
                    ];
                    shapes.push(Shape::Path(PathShape::line(pts, Stroke::new(2.0, p.on_primary))));
                } else {
                    shapes.push(Shape::rect_filled(r, 4.0, p.surface));
                    shapes.push(Shape::rect_stroke(r, 4.0, Stroke::new(1.0, p.border_strong)));
                }
            }
            Check::Single => {
                let c = r.center();
                if selected {
                    shapes.push(Shape::circle_stroke(c, 7.0, Stroke::new(1.5, p.primary)));
                    shapes.push(Shape::circle_filled(c, 4.0, p.primary));
                } else {
                    shapes.push(Shape::circle_filled(c, 7.0, p.surface));
                    shapes.push(Shape::circle_stroke(c, 7.0, Stroke::new(1.0, p.border_strong)));
                }
            }
            Check::None => {}
        }
    }
    ui.painter().set(bg, Shape::Vec(shapes));
    if check == Check::None { resp } else { resp.on_hover_cursor(CursorIcon::PointingHand) }
}

pub fn empty_state(ui: &mut Ui, title: &str, desc: &str, action: Option<&str>) -> Option<Response> {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(32.0);
        ui.label(RichText::new(title).font(bold(16.0)).color(p.text));
        ui.label(RichText::new(desc).color(p.text_2));
        let resp = action.map(|label| {
            ui.add_space(8.0);
            button(ui, label, Kind::Primary)
        });
        ui.add_space(32.0);
        resp
    })
    .inner
}

/// 提示块：soft 底 + 主色字，右侧可选一个按钮，返回按钮是否被点击
pub fn callout(ui: &mut Ui, text: &str, tone: Tone, action: Option<&str>) -> bool {
    let p = palette(ui.ctx());
    let (fg, bg) = p.tone(tone);
    let mut clicked = false;
    egui::Frame::none()
        .fill(bg)
        .rounding(8.0)
        .inner_margin(Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // 外包一层 horizontal：直接在纵向布局里用居中的横向子布局会占满剩余高度
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(label) = action {
                        clicked = button_ex(ui, label, Kind::Secondary, Size::Sm, false, true).clicked();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).color(fg)).wrap(true));
                    });
                });
            });
        });
    clicked
}

/// 可关闭的公告条，返回是否点了“关闭”
pub fn notice_bar(ui: &mut Ui, text: &str, tone: Tone) -> bool {
    let p = palette(ui.ctx());
    let (fg, bg) = p.tone(tone);
    let mut closed = false;
    egui::Frame::none()
        .fill(bg)
        .rounding(8.0)
        .inner_margin(Margin::symmetric(12.0, 6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    closed = button_ex(ui, "关闭", Kind::Ghost, Size::Sm, false, true).clicked();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).color(fg)).wrap(true));
                    });
                });
            });
        });
    closed
}

pub fn skeleton(ui: &mut Ui, w: f32, h: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, p.sunken);
}

/// 票根撕线：虚线 + 上下两个半圆缺口，top / bottom 是卡片上下边上的圆心
pub fn perforation(painter: &Painter, top: Pos2, bottom: Pos2, p: &Palette) {
    let r = 8.0;
    painter.extend(Shape::dashed_line(
        &[pos2(top.x, top.y + r + 3.0), pos2(bottom.x, bottom.y - r - 3.0)],
        Stroke::new(2.0, p.border_strong),
        6.0,
        4.0,
    ));
    // 缺口外半圆落在页面底色上，只给卡片内的半圆描边
    for (center, dir) in [(top, 1.0), (bottom, -1.0)] {
        painter.circle_filled(center, r, p.bg);
        let pts = (0..=16)
            .map(|i| {
                let a = PI * i as f32 / 16.0;
                pos2(center.x + r * a.cos(), center.y + dir * r * a.sin())
            })
            .collect();
        painter.add(PathShape::line(pts, Stroke::new(1.0, p.border)));
    }
}

/// 圆形头像；没有纹理时画占位圆
pub fn avatar(ui: &mut Ui, texture: Option<&TextureHandle>, size: f32) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), size / 2.0, p.sunken);
        if let Some(texture) = texture {
            egui::Image::from_texture(SizedTexture::from_handle(texture))
                .rounding(size / 2.0)
                .paint_at(ui, rect);
        }
    }
    resp
}

pub fn countdown_text(secs: i64) -> String {
    let s = secs.max(0);
    let hms = format!("{:02}:{:02}:{:02}", s % 86400 / 3600, s % 3600 / 60, s % 60);
    if s >= 86400 { format!("{} 天 {}", s / 86400, hms) } else { hms }
}

/// 分 → “¥1,360.00”
pub fn money(fen: i64) -> String {
    let sign = if fen < 0 { "-" } else { "" };
    let fen = fen.abs();
    let yuan = (fen / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in yuan.chars().enumerate() {
        if i > 0 && (yuan.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}¥{grouped}.{:02}", fen % 100)
}

pub fn mask_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 6 {
        return id.to_owned();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}{}{tail}", "*".repeat((chars.len() - 6).min(6)))
}

pub fn mask_phone(phone: &str) -> String {
    if phone.len() == 11 && phone.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}****{}", &phone[..3], &phone[7..])
    } else {
        phone.to_owned()
    }
}

pub fn copy_text(ctx: &egui::Context, text: impl Into<String>) {
    let text = text.into();
    ctx.output_mut(|o| o.copied_text = text);
}

pub fn open_url(ctx: &egui::Context, url: &str) {
    ctx.output_mut(|o| o.open_url = Some(egui::output::OpenUrl::new_tab(url)));
}

/// 生成二维码纹理（四周留 2 格白边），按 key 缓存，内容不变不重建
pub fn qr_texture(ctx: &egui::Context, key: &str, content: &str) -> TextureHandle {
    let id = egui::Id::new(("qr_texture", key));
    if let Some((cached, texture)) = ctx.data(|d| d.get_temp::<(String, TextureHandle)>(id)) {
        if cached == content {
            return texture;
        }
    }
    // 内容是程序生成的登录或支付链接，长度远小于二维码容量
    let code = QrCode::new(content.as_bytes()).unwrap();
    let n = code.width();
    let quiet = 2;
    let size = n + quiet * 2;
    let mut image = egui::ColorImage::new([size, size], Color32::WHITE);
    for (i, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            image.pixels[(i / n + quiet) * size + i % n + quiet] = Color32::BLACK;
        }
    }
    let texture = ctx.load_texture(format!("qr_{key}"), image, egui::TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(id, (content.to_owned(), texture.clone())));
    texture
}
