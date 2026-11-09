use std::sync::Arc;

use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Align, Color32, CursorIcon, FontId, Id, Label, Layout,
    Margin, Rect, Response, RichText, Sense, Stroke, TextureHandle, Ui,
};

use common::cookie_manager::CookieManager;
use common::show_orderlist::Order;
use common::ticket::BuyerInfo;
use common::utils::load_texture_from_url;

use crate::app::{AccountsTab, Dialog, Myapp};
use crate::state::remote::Remote;
use crate::ui::theme::{bold, palette, Palette, Tone};
use crate::ui::widgets::{self, button_ex, Check, Kind, Size};

const ORDER_DETAIL_URL: &str = "https://mall.bilibili.com/neul-next/ticket/orderDetail.html?order_id=";

pub fn open_order_detail(ctx: &egui::Context, order_id: &str) {
    widgets::open_url(ctx, &format!("{ORDER_DETAIL_URL}{order_id}"));
}

pub fn render(app: &mut Myapp, ui: &mut Ui) {
    let p = palette(ui.ctx());
    if app.account_manager.accounts.is_empty() {
        ui.add_space((ui.available_height() * 0.3 - 60.0).max(0.0));
        let action = widgets::empty_state(
            ui,
            "登录 B 站账号",
            "抢票需要至少一个已登录账号，支持扫码、短信和 Cookie 登录。",
            Some("扫码登录"),
        );
        if action.map_or(false, |r| r.clicked()) {
            app.open_login();
        }
        return;
    }

    // 选中的账号可能已退出，回落到第一个
    let uid = match app.accounts_selected {
        Some(uid) if app.account(uid).is_some() => uid,
        _ => app.account_manager.accounts[0].uid,
    };
    app.accounts_selected = Some(uid);

    egui::SidePanel::left("account_list")
        .exact_width(260.0)
        .resizable(false)
        .frame(egui::Frame::none().fill(p.bg).inner_margin(Margin { left: 10.0, right: 10.0, top: 16.0, bottom: 16.0 }))
        .show_inside(ui, |ui| account_list(app, ui, &p, uid));
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(p.bg).inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 20.0 }))
        .show_inside(ui, |ui| detail(app, ui, &p, uid));
}

fn account_list(app: &mut Myapp, ui: &mut Ui, p: &Palette, selected: i64) {
    let mut clicked = None;
    let mut add = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for account in &app.account_manager.accounts {
            let meta = format!("UID {} · LV{}", account.uid, account.level);
            let resp = account_row(ui, p, app.avatar_of(account.uid), &account.name, &meta, account.uid == selected);
            if resp.clicked() {
                clicked = Some(account.uid);
            }
        }
        ui.add_space(8.0);
        add = button_ex(ui, "+ 添加账号", Kind::Secondary, Size::Xl, false, true).clicked();
    });
    if let Some(uid) = clicked {
        app.accounts_selected = Some(uid);
    }
    if add {
        app.open_login();
    }
}

/// 账号列表行：高 56、圆角 8，选中 primary_soft 底
fn account_row(ui: &mut Ui, p: &Palette, avatar: Option<&TextureHandle>, name: &str, meta: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            p.primary_soft
        } else if resp.hovered() {
            p.sunken
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 8.0, fill);
        if resp.has_focus() {
            ui.painter().rect_stroke(rect.shrink(1.0), 8.0, Stroke::new(2.0, p.primary));
        }
        let inner = rect.shrink2(vec2(10.0, 8.0));
        let avatar_rect = Rect::from_min_size(pos2(inner.left(), inner.center().y - 16.0), vec2(32.0, 32.0));
        let mut avatar_ui = ui.child_ui(avatar_rect, Layout::left_to_right(Align::Center));
        widgets::avatar(&mut avatar_ui, avatar, 32.0);
        // 文字区固定在头像右侧并垂直居中，长昵称单行截断
        let text_rect = Rect::from_min_max(
            pos2(avatar_rect.right() + 10.0, inner.center().y - 18.0),
            pos2(inner.right(), inner.center().y + 18.0),
        );
        let mut text_ui = ui.child_ui(text_rect, Layout::top_down(Align::Min));
        text_ui.spacing_mut().item_spacing.y = 2.0;
        text_ui.add(Label::new(RichText::new(name).color(p.text)).truncate(true));
        text_ui.add(Label::new(RichText::new(meta).size(12.0).color(p.text_3)).truncate(true));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn detail(app: &mut Myapp, ui: &mut Ui, p: &Palette, uid: i64) {
    let Some(account) = app.account(uid) else { return };
    let name = account.name.clone();
    let level = account.level.clone();
    let vip = account.vip_label.clone();

    let mut copy_uid = false;
    let mut sign_out = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        widgets::avatar(ui, app.avatar_of(uid), 56.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.label(RichText::new(&name).font(bold(18.0)).color(p.text));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let uid_label = ui.add(
                    Label::new(RichText::new(format!("UID {uid}")).font(FontId::monospace(12.0)).color(p.text_3))
                        .sense(Sense::click()),
                );
                copy_uid = uid_label.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("点击复制").clicked();
                widgets::pill(ui, format!("LV{level}"), Tone::Neutral);
                if !vip.is_empty() {
                    widgets::pill(ui, vip.as_str(), Tone::Primary);
                }
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            sign_out = button_ex(ui, "退出登录", Kind::DangerGhost, Size::Md, false, true).clicked();
        });
    });
    if copy_uid {
        widgets::copy_text(ui.ctx(), uid.to_string());
        app.toasts.neutral("已复制 UID");
    }
    if sign_out {
        app.dialog = Dialog::ConfirmSignOut { uid, busy: false };
    }

    ui.add_space(10.0);
    tab_bar(ui, p, &mut app.accounts_tab);
    ui.add_space(4.0);
    match app.accounts_tab {
        AccountsTab::Buyers => buyers_tab(app, ui, p, uid),
        AccountsTab::Orders => orders_tab(app, ui, p, uid),
    }
}

/// 下划线式页签：选中 text 字 + 底部 2px primary，整行底部 1px border
fn tab_bar(ui: &mut Ui, p: &Palette, tab: &mut AccountsTab) {
    let mut selected_rect = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for (value, label) in [(AccountsTab::Buyers, "购票人"), (AccountsTab::Orders, "订单")] {
            let selected = *tab == value;
            let font = FontId::proportional(14.0);
            let w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), p.text).size().x;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
            let color = if selected || resp.hovered() { p.text } else { p.text_2 };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, color);
            if resp.has_focus() {
                ui.painter().rect_stroke(rect.expand(2.0), 4.0, Stroke::new(2.0, p.primary));
            }
            if selected {
                selected_rect = Some(rect);
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                *tab = value;
            }
        }
    });
    let bottom = row.response.rect.bottom();
    let x = ui.max_rect().x_range();
    ui.painter().hline(x, bottom - 0.5, Stroke::new(1.0, p.border));
    if let Some(rect) = selected_rect {
        let bar = Rect::from_min_max(pos2(rect.left(), bottom - 2.0), pos2(rect.right(), bottom));
        ui.painter().rect_filled(bar, 0.0, p.primary);
    }
}

fn refresh_label(cooldown: u64) -> String {
    if cooldown > 0 { format!("{cooldown} 秒后可刷新") } else { "刷新".to_owned() }
}

/// 失败行：danger 文字 + 重试，返回是否点了重试
fn failed_row(ui: &mut Ui, p: &Palette, text: &str) -> bool {
    let mut retry = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(text).color(p.danger));
        retry = button_ex(ui, "重试", Kind::Ghost, Size::Sm, false, true).clicked();
    });
    retry
}

fn buyers_tab(app: &mut Myapp, ui: &mut Ui, p: &Palette, uid: i64) {
    if matches!(app.buyers.get(&uid), None | Some(Remote::Idle)) {
        app.request_buyers(uid);
    }
    let cooldown = app.buyers.get(&uid).map_or(0, |r| r.cooldown_left(5));
    let from_cache = matches!(app.buyers.get(&uid), Some(Remote::Ready { value, .. }) if value.from_cache);

    let mut refresh = false;
    let mut add = false;
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        if from_cache {
            widgets::small(ui, "网络获取失败，正在使用本地缓存的购票人列表", p.warning);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            add = button_ex(ui, "+ 添加购票人", Kind::Secondary, Size::Sm, false, true).clicked();
            refresh = button_ex(ui, &refresh_label(cooldown), Kind::Ghost, Size::Sm, false, cooldown == 0).clicked();
        });
    });

    let mut retry = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.buyers.get(&uid) {
        Some(Remote::Ready { value, .. }) if value.list.is_empty() => {
            let action = widgets::empty_state(ui, "这个账号还没有购票人", "实名票需要先添加购票人", Some("添加购票人"));
            if action.map_or(false, |r| r.clicked()) {
                add = true;
            }
        }
        Some(Remote::Ready { value, .. }) => buyer_grid(ui, p, &value.list),
        Some(Remote::Failed { message, .. }) => {
            retry = failed_row(ui, p, &format!("获取购票人失败：{message}"));
        }
        _ => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for _ in 0..3 {
                    widgets::skeleton(ui, 220.0, 72.0);
                }
            });
        }
    });

    if refresh || retry {
        app.request_buyers(uid);
    }
    if add {
        app.open_add_buyer(uid);
    }
}

/// 与抢票页多选卡片同样式，但只读
fn buyer_grid(ui: &mut Ui, p: &Palette, list: &[BuyerInfo]) {
    let gap = 12.0;
    let avail = ui.available_width();
    let cols = (avail / 220.0).floor().max(1.0) as usize;
    let width = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
    ui.spacing_mut().item_spacing.y = gap;
    for row in list.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for buyer in row {
                widgets::select_card(ui, false, Check::None, width, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(&buyer.name).color(p.text));
                        if buyer.is_default == 1 {
                            widgets::pill(ui, "默认", Tone::Neutral);
                        }
                        widgets::small(ui, id_type_name(buyer.id_type), p.text_3);
                    });
                    let mono = |text: String| RichText::new(text).font(FontId::monospace(12.0)).color(p.text_2);
                    ui.label(mono(widgets::mask_id(&buyer.personal_id)));
                    ui.label(mono(widgets::mask_phone(&buyer.tel)));
                });
            }
        });
    }
}

fn id_type_name(id_type: i64) -> &'static str {
    match id_type {
        0 => "身份证",
        1 => "护照",
        2 => "港澳通行证",
        3 => "台湾通行证",
        _ => "其他证件",
    }
}

fn orders_tab(app: &mut Myapp, ui: &mut Ui, p: &Palette, uid: i64) {
    if matches!(app.orders.get(&uid), None | Some(Remote::Idle)) {
        app.request_orders(uid);
    }
    let cooldown = app.orders.get(&uid).map_or(0, |r| r.cooldown_left(5));
    let count = app.orders.get(&uid).and_then(|r| r.value()).map(|o| o.data.list.len());

    let mut refresh = false;
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        if let Some(n) = count {
            widgets::small(ui, format!("共 {n} 个订单"), p.text_3);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            refresh = button_ex(ui, &refresh_label(cooldown), Kind::Ghost, Size::Sm, false, cooldown == 0).clicked();
        });
    });

    // 封面图沿用旧版：用当前账号的 cookie_manager 下载
    let cookie_manager = app.account(uid).and_then(|a| a.cookie_manager.clone());
    let mut retry = false;
    let mut copied = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match app.orders.get(&uid) {
            Some(Remote::Ready { value, .. }) if value.data.list.is_empty() => {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    ui.label(RichText::new("还没有订单").color(p.text_2));
                });
            }
            Some(Remote::Ready { value, .. }) => {
                for order in &value.data.list {
                    if order_row(ui, p, order, cookie_manager.as_ref()) {
                        copied = Some(order.order_id.clone());
                    }
                }
            }
            Some(Remote::Failed { message, .. }) => {
                ui.add_space(8.0);
                let text = if message.is_empty() { "获取订单失败".to_owned() } else { format!("获取订单失败：{message}") };
                retry = failed_row(ui, p, &text);
            }
            _ => {
                for _ in 0..4 {
                    order_skeleton(ui, p);
                }
            }
        }
    });

    if refresh || retry {
        app.request_orders(uid);
    }
    if let Some(order_id) = copied {
        widgets::copy_text(ui.ctx(), order_id);
        app.toasts.neutral("已复制订单号");
    }
}

/// 订单行（高 68，下边 1px border）。返回是否点了“复制订单号”
fn order_row(ui: &mut Ui, p: &Palette, order: &Order, cookie_manager: Option<&Arc<CookieManager>>) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let mut row = ui.child_ui(rect.shrink2(vec2(0.0, 10.0)), Layout::left_to_right(Align::Center));
    row.spacing_mut().item_spacing.x = 14.0;

    let (cover, _) = row.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    let texture = cover_texture(row.ctx(), cookie_manager, &order.img.url);
    match texture {
        Some(texture) => egui::Image::from_texture(SizedTexture::from_handle(&texture))
            .rounding(6.0)
            .paint_at(&mut row, cover),
        None => row.painter().rect_filled(cover, 6.0, p.sunken),
    }

    let mut copy = false;
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let pending = order.status == 1 && order.sub_status == 1;
        let open = if pending {
            outline_primary_button(ui, p, "在浏览器打开")
        } else {
            button_ex(ui, "在浏览器打开", Kind::Ghost, Size::Sm, false, true)
        };
        if open.clicked() {
            open_order_detail(ui.ctx(), &order.order_id);
        }
        copy = button_ex(ui, "复制订单号", Kind::Ghost, Size::Sm, false, true).clicked();
        ui.add_space(10.0);

        let tone = if pending {
            Tone::Warning
        } else if order.status == 2 {
            Tone::Success
        } else {
            Tone::Neutral
        };
        ui.allocate_ui_with_layout(vec2(90.0, 28.0), Layout::left_to_right(Align::Center), |ui| {
            widgets::pill(ui, order.sub_status_name.as_str(), tone);
        });
        ui.allocate_ui_with_layout(vec2(110.0, 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.label(RichText::new(widgets::money(order.pay_money)).font(FontId::monospace(13.0)).color(p.text));
        });
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.add_space(5.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(Label::new(RichText::new(&order.item_info.name).color(p.text)).truncate(true));
            let meta = format!("{} · 下单 {}", order.item_info.screen_name, order.ctime);
            ui.add(Label::new(RichText::new(meta).size(12.0).color(p.text_3)).truncate(true));
        });
    });
    copy
}

fn order_skeleton(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::hover());
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let cover = Rect::from_min_size(pos2(rect.left(), rect.center().y - 24.0), vec2(48.0, 48.0));
    painter.rect_filled(cover, 6.0, p.sunken);
    let x = cover.right() + 14.0;
    painter.rect_filled(Rect::from_min_size(pos2(x, rect.center().y - 16.0), vec2(220.0, 14.0)), 6.0, p.sunken);
    painter.rect_filled(Rect::from_min_size(pos2(x, rect.center().y + 4.0), vec2(150.0, 12.0)), 6.0, p.sunken);
}

/// 待付款订单的“在浏览器打开”：Secondary 形状，primary 描边和文字
fn outline_primary_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), p.primary);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() || resp.is_pointer_button_down_on() { p.primary_soft } else { p.surface };
        ui.painter().rect_filled(rect, 6.0, fill);
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, p.primary));
        if resp.has_focus() {
            ui.painter().rect_stroke(rect.expand(2.0), 8.0, Stroke::new(2.0, p.primary));
        }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// 订单封面，沿用旧版加载方式：后台线程下载缩略图，纹理按图片地址存进 ctx 临时数据。
/// 失败时保留“加载中”标记，避免每帧重新发起下载；封面一直显示占位
fn cover_texture(ctx: &egui::Context, cookie_manager: Option<&Arc<CookieManager>>, raw_url: &str) -> Option<TextureHandle> {
    if raw_url.is_empty() {
        return None;
    }
    let url = if raw_url.starts_with("//") { format!("https:{raw_url}") } else { raw_url.to_owned() };
    let id = Id::new(&url);
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(texture);
    }
    let loading = Id::new(format!("loading_{url}"));
    if ctx.data(|d| d.get_temp::<bool>(loading)).is_some() {
        return None;
    }
    let cookie_manager = cookie_manager?.clone();
    ctx.data_mut(|d| d.insert_temp(loading, true));
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        match load_texture_from_url(&ctx, cookie_manager, &format!("{url}@74w_74h.jpeg"), &url) {
            Some(texture) => {
                ctx.data_mut(|d| d.insert_temp(id, texture));
                ctx.request_repaint();
            }
            None => log::warn!("加载订单封面失败: {}", url),
        }
    });
    None
}
