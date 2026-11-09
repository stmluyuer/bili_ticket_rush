use eframe::egui::{
    self, pos2, vec2, Align, CursorIcon, FontId, Layout, Margin, Rect, Response, RichText, Sense,
    Shape, Stroke, Ui,
};

use common::push::PushConfig;
use common::taskmanager::{PushRequest, PushType, TaskRequest};

use crate::app::{Myapp, PushTest};
use crate::state::persist::{SaveState, ThemeMode};
use crate::ui::theme::{palette, Palette, Tone};
use crate::ui::widgets::{
    button_ex, card, page_title, section_title, segmented, small, switch, text_field, FieldOpts,
    Filter, Kind, Size,
};

const LABEL_W: f32 = 160.0;

// 保留样本中的设备、系统和内核版本组合；小米项仅保留 WebView 基础值，不含库街区后缀。
const UA_PRESETS: [(&str, &str); 3] = [
    (
        "OPPO Find X6 Pro · Android 16",
        "Mozilla/5.0 (Linux; Android 16; PGEM10 Build/BP2A.250605.015; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/145.0.7632.79 Mobile Safari/537.36",
    ),
    (
        "小米 14 · Android 14",
        "Mozilla/5.0 (Linux; Android 14; 23127PN0CC Build/UKQ1.230804.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/127.0.6533.2 Mobile Safari/537.36",
    ),
    (
        "vivo X100 · Android 14",
        "Mozilla/5.0 (Linux; Android 14; V2309A) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.200 Mobile Safari/537.36 VivoBrowser/28.0.0.0",
    ),
];

/// 与 app.settings_expanded 的下标一一对应
const CHANNELS: [&str; 7] = ["Bark", "PushPlus", "Server 酱", "钉钉机器人", "企业微信", "邮件（SMTP）", "Gotify"];

pub fn render(app: &mut Myapp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    egui::ScrollArea::vertical()
        .id_source("settings_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 24.0 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(720.0));
                    header(app, ui, &p);
                    ui.add_space(8.0);

                    let mut dirty = false;
                    dirty |= appearance(app, ui, &p);
                    ui.add_space(8.0);
                    dirty |= captcha(app, ui, &p);
                    ui.add_space(8.0);
                    dirty |= network(app, ui, &p);
                    ui.add_space(8.0);
                    dirty |= push(app, ui, &p);
                    if dirty {
                        app.mark_settings_dirty();
                    }
                });
        });
}

/// 页头：标题 + 自动保存状态
fn header(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let state = app.settings_save.clone();
    // 自动保存有 800ms 延迟，等待期间也显示为保存中
    let pending = app.settings_dirty_at.is_some();
    let mut retry = false;
    page_title(ui, "设置", |ui| {
        if pending {
            small(ui, "保存中…", p.text_3);
            return;
        }
        match &state {
            SaveState::Idle => {}
            SaveState::Saving => {
                small(ui, "保存中…", p.text_3);
            }
            SaveState::Saved => {
                small(ui, "已保存", p.text_3);
            }
            SaveState::Failed(message) => {
                retry = button_ex(ui, "重试", Kind::Ghost, Size::Sm, false, true).clicked();
                small(ui, "保存失败", p.danger).on_hover_text(message.as_str());
            }
        }
    });
    if retry {
        app.save_settings_now();
    }
}

fn appearance(app: &mut Myapp, ui: &mut Ui, p: &Palette) -> bool {
    let mut changed = false;
    card(ui, |ui| {
        section_title(ui, "外观", |_| {});
        row(ui, p, "主题", |ui| {
            changed |= segmented(
                ui,
                &mut app.ui_config.theme,
                &[(ThemeMode::System, "跟随系统"), (ThemeMode::Light, "浅色"), (ThemeMode::Dark, "深色")],
            );
        });
        divider(ui, p);
        row(ui, p, "界面缩放", |ui| {
            changed |= segmented(
                ui,
                &mut app.ui_config.scale,
                &[(0.9, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%")],
            );
        });
    });
    changed
}

fn captcha(app: &mut Myapp, ui: &mut Ui, p: &Palette) -> bool {
    let mut changed = false;
    card(ui, |ui| {
        section_title(ui, "验证码", |_| {});
        // 旧配置可能存了已去掉的选项 2：显示为本地识别，用户点选前不改写
        let mut mode: usize = if app.custom_config.captcha_mode == 1 { 1 } else { 0 };
        row(ui, p, "识别方式", |ui| {
            if segmented(ui, &mut mode, &[(0, "本地识别"), (1, "TTOCR")]) {
                app.custom_config.captcha_mode = mode;
                changed = true;
            }
        });
        if app.custom_config.captcha_mode == 1 {
            divider(ui, p);
            row(ui, p, "TTOCR Key", |ui| {
                changed |= field(ui, "ttocr_key", None, "填写 TTOCR 的 Key", &mut app.custom_config.ttocr_key, true, Filter::Token);
            });
        }
    });
    changed
}

fn network(app: &mut Myapp, ui: &mut Ui, p: &Palette) -> bool {
    let mut changed = false;
    card(ui, |ui| {
        section_title(ui, "网络", |_| {});
        row(ui, p, "自定义 UA", |ui| {
            changed |= switch(ui, &mut app.custom_config.open_custom_ua).changed();
        });
        let enabled = app.custom_config.open_custom_ua;
        ui.horizontal_top(|ui| {
            ui.add_space(LABEL_W + 8.0);
            ui.add_enabled_ui(enabled, |ui| {
                // 禁用区域继承外层横排布局，这里显式纵排，避免输入框被候选框挤窄。
                ui.set_layout(Layout::top_down(Align::Min));
                let selected = UA_PRESETS.iter()
                    .find(|(_, ua)| *ua == app.custom_config.custom_ua)
                    .map(|(label, _)| *label)
                    .unwrap_or("自定义");
                egui::ComboBox::from_id_source("custom_ua_preset")
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (label, ua) in UA_PRESETS {
                            changed |= ui.selectable_value(
                                &mut app.custom_config.custom_ua,
                                ua.to_owned(),
                                label,
                            ).changed();
                        }
                    });
                let resp = text_field(
                    ui,
                    &mut app.custom_config.custom_ua,
                    FieldOpts {
                        hint: "填写完整的 User-Agent",
                        multiline_rows: 3,
                        id: Some("custom_ua"),
                        ..Default::default()
                    },
                );
                changed |= resp.changed();
            });
        });
    });
    changed
}

fn push(app: &mut Myapp, ui: &mut Ui, p: &Palette) -> bool {
    let mut changed = false;
    let mut send_test = false;
    card(ui, |ui| {
        section_title(ui, "推送", |_| {});
        row(ui, p, "抢到票时发送通知", |ui| {
            changed |= switch(ui, &mut app.push_config.enabled).changed();
        });
        if !app.push_config.enabled {
            return;
        }
        divider(ui, p);
        for (i, name) in CHANNELS.iter().enumerate() {
            let open = app.settings_expanded[i];
            if channel_header(ui, p, name, channel_filled(&app.push_config, i), open).clicked() {
                app.settings_expanded[i] = !open;
            }
            if app.settings_expanded[i] {
                egui::Frame::none()
                    .inner_margin(Margin { left: 12.0, right: 12.0, top: 0.0, bottom: 8.0 })
                    .show(ui, |ui| {
                        changed |= channel_fields(ui, &mut app.push_config, i);
                    });
            }
            divider(ui, p);
        }
        ui.horizontal(|ui| {
            small(ui, "修改只对之后新建的任务生效", p.text_3);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let busy = matches!(app.push_test, PushTest::Busy(_));
                send_test = button_ex(ui, "发送测试推送", Kind::Secondary, Size::Sm, busy, true).clicked();
                match &app.push_test {
                    PushTest::Sent => {
                        small(ui, "测试推送已发送", p.success);
                    }
                    PushTest::Failed(message) => {
                        let text = format!("发送失败：{message}");
                        ui.add(egui::Label::new(RichText::new(text.as_str()).size(12.0).color(p.danger)).truncate(true))
                            .on_hover_text(text);
                    }
                    PushTest::Idle | PushTest::Busy(_) => {}
                }
            });
        });
    });
    if send_test {
        // push_all 不返回任务 ID，这里提交与它相同的请求，按任务 ID 匹配结果
        let request = TaskRequest::PushRequest(PushRequest {
            title: "biliticket推送测试".into(),
            message: "这是一个推送测试".into(),
            jump_url: None,
            push_config: app.push_config.clone(),
            push_type: PushType::All,
        });
        app.push_test = match app.task_manager.submit_task(request) {
            Ok(task_id) => {
                log::debug!("提交测试推送任务成功，任务ID: {}", task_id);
                PushTest::Busy(task_id)
            }
            Err(e) => {
                log::error!("提交测试推送任务失败: {}", e);
                PushTest::Failed(e)
            }
        };
    }
    changed
}

fn channel_filled(cfg: &PushConfig, i: usize) -> bool {
    let smtp = &cfg.smtp_config;
    let gotify = &cfg.gotify_config;
    let fields: Vec<&String> = match i {
        0 => vec![&cfg.bark_token],
        1 => vec![&cfg.pushplus_token],
        2 => vec![&cfg.fangtang_token],
        3 => vec![&cfg.dingtalk_token],
        4 => vec![&cfg.wechat_token],
        5 => vec![&smtp.smtp_server, &smtp.smtp_port, &smtp.smtp_username, &smtp.smtp_password, &smtp.smtp_from, &smtp.smtp_to],
        _ => vec![&gotify.gotify_url, &gotify.gotify_token],
    };
    fields.iter().any(|f| !f.is_empty())
}

fn channel_fields(ui: &mut Ui, cfg: &mut PushConfig, i: usize) -> bool {
    match i {
        0 => field(ui, "push_bark", Some("Token"), "只填 token，可用 ? 追加参数", &mut cfg.bark_token, true, Filter::None),
        1 => field(ui, "push_pushplus", Some("Token"), "PushPlus 的 token", &mut cfg.pushplus_token, true, Filter::Token),
        2 => field(ui, "push_fangtang", Some("SendKey"), "SCT 开头", &mut cfg.fangtang_token, true, Filter::Token),
        3 => field(ui, "push_dingtalk", Some("Token"), "钉钉机器人的 access_token", &mut cfg.dingtalk_token, true, Filter::Token),
        4 => field(ui, "push_wechat", Some("机器人 Key"), "企业微信群机器人的 key", &mut cfg.wechat_token, true, Filter::Token),
        5 => {
            let s = &mut cfg.smtp_config;
            let mut changed = false;
            changed |= field(ui, "smtp_server", Some("服务器"), "例如 smtp.qq.com", &mut s.smtp_server, false, Filter::Token);
            changed |= field(ui, "smtp_port", Some("端口"), "例如 465", &mut s.smtp_port, false, Filter::Digits);
            changed |= field(ui, "smtp_username", Some("邮箱账号"), "登录 SMTP 的邮箱", &mut s.smtp_username, false, Filter::Token);
            changed |= field(ui, "smtp_password", Some("授权码"), "邮箱的 SMTP 授权码", &mut s.smtp_password, true, Filter::Token);
            changed |= field(ui, "smtp_from", Some("发件人"), "发件人邮箱", &mut s.smtp_from, false, Filter::Token);
            changed |= field(ui, "smtp_to", Some("收件人"), "收件人邮箱", &mut s.smtp_to, false, Filter::Token);
            changed
        }
        _ => {
            let g = &mut cfg.gotify_config;
            let mut changed = false;
            changed |= field(ui, "gotify_url", Some("服务器地址"), "只填服务器地址", &mut g.gotify_url, false, Filter::None);
            changed |= field(ui, "gotify_token", Some("Token"), "Gotify 应用的 token", &mut g.gotify_token, true, Filter::Token);
            changed
        }
    }
}

/// 输入框最宽 420；ID 固定，展开 / 收起渠道时焦点不会串到别的输入框
fn field(ui: &mut Ui, id: &str, label: Option<&str>, hint: &str, value: &mut String, password: bool, filter: Filter) -> bool {
    let width = ui.available_width().min(420.0);
    text_field(
        ui,
        value,
        FieldOpts { label, hint, password, filter, width: Some(width), id: Some(id), ..Default::default() },
    )
    .changed()
}

/// 两列行：左侧 160 宽标签（text_2），右侧控件
fn row(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(LABEL_W, 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_size(vec2(LABEL_W, 32.0));
            ui.label(RichText::new(label).color(p.text_2));
        });
        add(ui);
    });
}

fn divider(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
}

/// 推送渠道行：名称、填写状态标签、右侧展开箭头；整行可点击
fn channel_header(ui: &mut Ui, p: &Palette, name: &str, filled: bool, open: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, 6.0, p.sunken);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.shrink(1.0), 6.0, Stroke::new(2.0, p.primary));
        }
        let name_galley = painter.layout_no_wrap(name.to_owned(), FontId::proportional(14.0), p.text);
        let name_size = name_galley.size();
        painter.galley(pos2(rect.left() + 8.0, rect.center().y - name_size.y / 2.0), name_galley);

        let (text, tone) = if filled { ("已填写", Tone::Success) } else { ("未填写", Tone::Neutral) };
        let (fg, bg) = p.tone(tone);
        let pill_galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), fg);
        let pill = Rect::from_min_size(
            pos2(rect.left() + 8.0 + name_size.x + 10.0, rect.center().y - 11.0),
            vec2(pill_galley.size().x + 16.0, 22.0),
        );
        painter.rect_filled(pill, 11.0, bg);
        painter.galley(pill.center() - pill_galley.size() / 2.0, pill_galley);

        let c = pos2(rect.right() - 16.0, rect.center().y);
        let pts = if open {
            vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)]
        } else {
            vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)]
        };
        painter.add(Shape::line(pts, Stroke::new(1.5, p.text_3)));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}
