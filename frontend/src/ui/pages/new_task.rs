use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone;
use eframe::egui::{
    self, pos2, vec2, Align, Align2, CursorIcon, FontId, Key, Label, Layout, Margin, Modifiers,
    Painter, Pos2, Rect, Response, RichText, Rounding, Sense, Stroke, Ui,
};

use common::taskmanager::{GrabTicketRequest, TaskManager_debug, TaskRequest, TaskStatus};
use common::ticket::{BilibiliTicket, BuyerInfo, NoBindBuyerInfo, ScreenInfo, ScreenTicketInfo, TicketInfo};

use crate::app::{Dialog, Myapp};
use crate::state::draft::{parse_project_input, ParseResult};
use crate::state::persist::Contact;
use crate::state::remote::Remote;
use crate::ui::theme::{bold, palette, Palette, Tone};
use crate::ui::toast::ToastAction;
use crate::ui::widgets::{self, button_ex, Check, FieldOpts, Filter, Kind, Size};

const MODE_NAMES: [&str; 3] = ["定时抢票", "直接下单", "捡漏回流"];
const MAX_BUYERS: usize = 10;

/// 购票人区的形态，由模式和项目的实名类型决定
#[derive(Clone, Copy, PartialEq, Eq)]
enum BuyerMode {
    /// 模式 0/1 且项目未加载，还不知道实名类型
    Pending,
    NoBind,
    Single,
    Multi,
    /// id_bind 缺失或不是 0/1/2，旧确认窗对此禁用提交
    Unknown,
}

pub fn render(app: &mut Myapp, ui: &mut Ui) {
    let p = palette(ui.ctx());

    ensure_account(app);
    let pasted = sync_input(app);
    sync_account(app, ui.ctx());
    if pasted && can_fetch(app) {
        app.fetch_project();
    }
    request_buyers_if_needed(app);
    normalize_selection(app);

    if matches!(app.dialog, Dialog::None)
        && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter))
        && missing(app).is_empty()
    {
        submit(app);
    }

    // 摘要栏要在中央内容之前声明，才能先占住右侧或底部
    if ui.ctx().screen_rect().width() >= 1100.0 {
        egui::SidePanel::right("summary")
            .exact_width(320.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::none().inner_margin(Margin { left: 0.0, right: 24.0, top: 20.0, bottom: 24.0 }))
            .show_inside(ui, |ui| summary_panel(app, ui, &p));
    } else {
        egui::TopBottomPanel::bottom("summary_bar")
            .resizable(false)
            .frame(egui::Frame::none().fill(p.surface).inner_margin(Margin::symmetric(24.0, 10.0)))
            .show_inside(ui, |ui| summary_bar(app, ui, &p));
    }

    egui::CentralPanel::default().frame(egui::Frame::none()).show_inside(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_source("new_task_form")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Frame::none()
                    .inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 24.0 })
                    .show(ui, |ui| {
                        // 最大宽度也不能超过侧栏分配后剩余的空间，否则会遮住任务摘要。
                        ui.set_max_width(ui.available_width().min(680.0));
                        ui.spacing_mut().item_spacing.y = 16.0;
                        header(app, ui, &p);
                        account_card(app, ui, &p);
                        mode_card(app, ui, &p);
                        if app.draft.mode == 2 {
                            scope_card(app, ui, &p);
                        } else {
                            ticket_card(app, ui, &p);
                        }
                        buyer_card(app, ui, &p);
                    });
            });
    });
}

// ---------------------------------------------------------------- 状态同步

/// 默认选第一个账号；所选账号已被删除时改选第一个
fn ensure_account(app: &mut Myapp) {
    if app.draft.account_uid.map_or(true, |uid| app.account(uid).is_none()) {
        app.draft.account_uid = app.account_manager.accounts.first().map(|a| a.uid);
    }
}

/// 每帧重新解析输入；返回本帧是否像一次粘贴（增长 ≥ 10 字符且识别出 ID）
fn sync_input(app: &mut Myapp) -> bool {
    let d = &mut app.draft;
    d.parsed = parse_project_input(d.project_input.trim());
    let len = d.project_input.chars().count();
    let pasted = len >= d.last_paste_len + 10 && matches!(d.parsed, ParseResult::Id { .. });
    d.last_paste_len = len;
    // 已有的项目信息属于另一个项目 ID 时作废，避免按旧项目提交
    if let Some((_, pid)) = &d.project_for {
        let same = matches!(&d.parsed, ParseResult::Id { id, .. } if id == pid);
        if !same {
            d.project = Remote::Idle;
            d.project_for = None;
            d.screen_idx = None;
            d.ticket_id = None;
        }
    }
    pasted
}

/// 账号变化（本页下拉、登录新账号、删除账号）时清空项目与购票人选择，并回填该账号的联系人
fn sync_account(app: &mut Myapp, ctx: &egui::Context) {
    let key = egui::Id::new("new_task_seen_account");
    let seen: Option<Option<i64>> = ctx.data(|d| d.get_temp(key));
    let now = app.draft.account_uid;
    if seen == Some(now) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, now));
    let contact = now.and_then(|uid| app.contacts.get(&uid.to_string()));
    if let Some(c) = contact {
        app.draft.contact_name = c.name.clone();
        app.draft.contact_tel = c.tel.clone();
    } else {
        app.draft.contact_name.clear();
        app.draft.contact_tel.clear();
    }
    // 首次进入本页不算切换
    if seen.is_none() {
        return;
    }
    let d = &mut app.draft;
    d.project = Remote::Idle;
    d.project_for = None;
    d.screen_idx = None;
    d.ticket_id = None;
    d.selected_buyers.clear();
    d.select_after_refresh = None;
    if now.is_some() && matches!(d.parsed, ParseResult::Id { .. }) {
        app.fetch_project();
    }
}

fn can_fetch(app: &Myapp) -> bool {
    app.draft.account_uid.is_some()
        && matches!(app.draft.parsed, ParseResult::Id { .. })
        && app.draft.project.cooldown_left(5) == 0
}

fn request_buyers_if_needed(app: &mut Myapp) {
    let Some(uid) = app.draft.account_uid else { return };
    let needed = app.draft.mode == 2 || matches!(project_bind(app), Some(Some(1 | 2)));
    if needed && matches!(app.buyers.get(&uid), None | Some(Remote::Idle)) {
        app.request_buyers(uid);
    }
}

fn normalize_selection(app: &mut Myapp) {
    // 定时模式下售罄、不可售的票种不可选
    if app.draft.mode == 0 && selected_ticket(app).map_or(false, |t| !timed_selectable(t)) {
        app.draft.ticket_id = None;
    }
    if buyer_mode(app) == BuyerMode::Single {
        app.draft.selected_buyers.truncate(1);
    }
    // 列表刷新后已不存在的购票人移出选择
    if let Some(list) = app.draft.account_uid.and_then(|uid| app.buyers.get(&uid)).and_then(|r| r.value()) {
        let ids: Vec<i64> = list.list.iter().map(|b| b.id).collect();
        app.draft.selected_buyers.retain(|id| ids.contains(id));
    }
}

// ---------------------------------------------------------------- 派生数据

/// None：项目未就绪；Some(id_bind)
fn project_bind(app: &Myapp) -> Option<Option<usize>> {
    app.draft.project.value().map(|p| p.id_bind)
}

fn buyer_mode(app: &Myapp) -> BuyerMode {
    if app.draft.mode == 2 {
        return BuyerMode::Multi;
    }
    match project_bind(app) {
        None => BuyerMode::Pending,
        Some(Some(0)) => BuyerMode::NoBind,
        Some(Some(1)) => BuyerMode::Single,
        Some(Some(2)) => BuyerMode::Multi,
        Some(_) => BuyerMode::Unknown,
    }
}

fn selected_screen(app: &Myapp) -> Option<&ScreenInfo> {
    let project = app.draft.project.value()?;
    project.screen_list.get(app.draft.screen_idx?)
}

fn selected_ticket(app: &Myapp) -> Option<&ScreenTicketInfo> {
    let id = app.draft.ticket_id?;
    selected_screen(app)?.ticket_list.iter().find(|t| t.id.unwrap_or(0) as i64 == id)
}

/// 定时模式可选：在售或未开售
fn timed_selectable(t: &ScreenTicketInfo) -> bool {
    t.clickable == Some(true) || t.sale_flag_number == Some(1)
}

fn nothing_on_sale(project: &TicketInfo) -> bool {
    project
        .screen_list
        .iter()
        .flat_map(|s| &s.ticket_list)
        .all(|t| t.clickable != Some(true) && t.sale_flag_number != Some(1))
}

/// 按选择顺序取出购票人
fn chosen_buyers(app: &Myapp) -> Vec<BuyerInfo> {
    let Some(list) = app.draft.account_uid.and_then(|uid| app.buyers.get(&uid)).and_then(|r| r.value()) else {
        return Vec::new();
    };
    app.draft
        .selected_buyers
        .iter()
        .filter_map(|id| list.list.iter().find(|b| b.id == *id))
        .cloned()
        .collect()
}

fn tel_valid(tel: &str) -> bool {
    tel.len() == 11 && tel.bytes().all(|b| b.is_ascii_digit())
}

fn missing(app: &Myapp) -> Vec<&'static str> {
    let d = &app.draft;
    let account = d.account_uid.and_then(|uid| app.account(uid));
    let mut out = Vec::new();
    if account.is_none() {
        out.push("登录账号");
    }
    if !matches!(d.parsed, ParseResult::Id { .. }) {
        out.push("输入项目 ID 或链接");
    }
    if d.mode != 2 {
        if d.project.value().is_none() {
            out.push("获取项目信息");
        }
        if selected_ticket(app).is_none() {
            out.push("选择票种");
        }
    }
    let chosen = chosen_buyers(app).len();
    match buyer_mode(app) {
        BuyerMode::Multi if chosen == 0 => out.push("至少选择 1 位购票人"),
        BuyerMode::Single if chosen == 0 => out.push("选择 1 位购票人"),
        BuyerMode::NoBind => {
            if d.contact_name.trim().is_empty() {
                out.push("填写联系人姓名");
            }
            if !tel_valid(&d.contact_tel) {
                out.push("填写 11 位手机号");
            }
        }
        BuyerMode::Unknown => out.push("该项目实名类型未知，暂不支持"),
        _ => {}
    }
    if d.mode == 2 && project_bind(app) == Some(Some(0)) {
        out.push("该项目不支持捡漏回流");
    }
    if account.map_or(false, |a| a.cookie_manager.is_none()) {
        out.push("重新登录该账号");
    }
    out
}

/// 与提交时写入 board.submitted 的 (uid, project_id, ticket_id) 相同
fn submit_key(app: &Myapp) -> Option<(i64, String, String)> {
    let uid = app.draft.account_uid?;
    if app.draft.mode == 2 {
        let ParseResult::Id { id, .. } = &app.draft.parsed else { return None };
        Some((uid, id.clone(), String::new()))
    } else {
        let project = app.draft.project.value()?;
        let ticket = selected_ticket(app)?;
        Some((uid, project.id.unwrap_or(0).to_string(), ticket.id.unwrap_or(0).to_string()))
    }
}

/// 进行中的相同任务的编号
fn duplicate_seq(app: &Myapp) -> Option<u64> {
    let (uid, pid, tid) = submit_key(app)?;
    app.board
        .running
        .iter()
        .find(|t| {
            app.board
                .submitted
                .iter()
                .any(|(task_id, u, p, k)| *task_id == t.task_id && *u == uid && *p == pid && *k == tid)
        })
        .map(|t| t.seq)
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn fmt_time(ts: i64, fmt: &str) -> String {
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format(fmt).to_string())
        .unwrap_or_default()
}

fn bind_label(id_bind: Option<usize>) -> &'static str {
    match id_bind {
        Some(0) => "非实名",
        Some(1) => "一单一证",
        Some(2) => "一人一证",
        _ => "实名类型未知",
    }
}

fn id_type_label(id_type: i64) -> &'static str {
    match id_type {
        0 => "身份证",
        1 => "护照",
        2 => "港澳通行证",
        3 => "台湾通行证",
        _ => "其他证件",
    }
}

/// performance_desc 中 base_info 模块的前 3 项“title：content”
fn base_info(project: &TicketInfo) -> String {
    let Some(desc) = &project.performance_desc else { return String::new() };
    desc.list
        .iter()
        .filter(|m| m.module == "base_info")
        .filter_map(|m| m.details.as_array())
        .flatten()
        .filter_map(|item| {
            Some(format!("{}：{}", item.get("title")?.as_str()?, item.get("content")?.as_str()?))
        })
        .take(3)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn ticket_status(t: &ScreenTicketInfo) -> (String, Tone) {
    if t.clickable == Some(true) {
        ("可售".into(), Tone::Success)
    } else if t.sale_flag_number == Some(1) {
        match t.saleStart.filter(|s| *s > 0) {
            Some(ts) => (format!("未开售 · {}", fmt_time(ts as i64, "%m-%d %H:%M")), Tone::Neutral),
            None => ("未开售".into(), Tone::Neutral),
        }
    } else {
        (t.sale_flag.display_name.clone(), Tone::Neutral)
    }
}

fn skipped(name: &str, words: &[String]) -> bool {
    words.iter().any(|w| name.contains(w.as_str()))
}

/// 基础层 callout / notice_bar 内部用 right_to_left(Center)，直接放在纵向布局里会占满剩余高度；放进一行里限制高度
fn in_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(add).inner
}

fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(size), ui.visuals().text_color()).size().x
}

// ---------------------------------------------------------------- 表单

fn header(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    widgets::small(ui, TaskManager_debug(), p.text_3);
    if let Some(text) = app.announce1.clone() {
        if !app.notice_dismissed[0] && in_row(ui, |ui| widgets::notice_bar(ui, &text, Tone::Info)) {
            app.notice_dismissed[0] = true;
        }
    }
}

fn account_card(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    widgets::card(ui, |ui| {
        if app.account_manager.accounts.is_empty() {
            let resp = widgets::empty_state(ui, "还没有登录账号", "登录后才能获取项目信息", Some("登录账号"));
            if resp.map_or(false, |r| r.clicked()) {
                app.open_login();
            }
            return;
        }
        widgets::section_title(ui, "账号与项目", |_| {});
        account_picker(app, ui);
        project_input(app, ui, p);
        project_status(app, ui, p);
    });
}

fn account_picker(app: &mut Myapp, ui: &mut Ui) {
    let selected = app.draft.account_uid;
    let mut chosen = selected;
    ui.horizontal(|ui| {
        // ComboBox 的选中文字不能带图片，头像放在下拉框左侧
        widgets::avatar(ui, selected.and_then(|uid| app.avatar_of(uid)), 20.0);
        let text = selected
            .and_then(|uid| app.account(uid))
            .map(|a| format!("{}  LV{}", a.name, a.level))
            .unwrap_or_else(|| "选择账号".into());
        egui::ComboBox::from_id_source("new_task_account")
            .width(240.0)
            .selected_text(text)
            .show_ui(ui, |ui| {
                for a in &app.account_manager.accounts {
                    if ui.selectable_label(Some(a.uid) == selected, format!("{}  UID {}", a.name, a.uid)).clicked() {
                        chosen = Some(a.uid);
                    }
                }
            });
    });
    if chosen != selected {
        app.draft.account_uid = chosen;
        sync_account(app, ui.ctx());
    }
}

fn project_input(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let cooldown = app.draft.project.cooldown_left(5);
    let loading = matches!(app.draft.project, Remote::Loading { .. });
    let busy = loading && cooldown > 0;
    let label = if cooldown > 0 && !loading { format!("{cooldown} 秒后可重试") } else { "获取项目".to_string() };
    let enabled = can_fetch(app);
    let button_w = text_width(ui, &label, 14.0) + 28.0 + if busy { 20.0 } else { 0.0 };
    let mut fetch = false;
    ui.horizontal(|ui| {
        let field_w = ui.available_width() - button_w - ui.spacing().item_spacing.x;
        let resp = widgets::text_field(
            ui,
            &mut app.draft.project_input,
            FieldOpts { hint: "项目 ID 或会员购链接", width: Some(field_w), id: Some("new_task_project"), ..Default::default() },
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            fetch = true;
        }
        if button_ex(ui, &label, Kind::Secondary, Size::Md, busy, enabled).clicked() {
            fetch = true;
        }
    });
    // 输入框本帧的改动要先重新解析，回车获取的才是新 ID
    if sync_input(app) {
        fetch = true;
    }
    let mut selected = None;
    let mut copied = None;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_source("new_task_project_history")
            .selected_text("历史项目")
            .width(160.0)
            .show_ui(ui, |ui| {
                if app.project_history.is_empty() {
                    ui.label("暂无历史记录");
                }
                for id in &app.project_history {
                    let current = matches!(&app.draft.parsed, ParseResult::Id { id: current, .. } if current == id);
                    if ui.selectable_label(current, id).clicked() {
                        selected = Some(id.clone());
                    }
                }
            });
        if let ParseResult::Id { id, .. } = &app.draft.parsed {
            if button_ex(ui, "复制编号", Kind::Secondary, Size::Sm, false, true).clicked() {
                widgets::copy_text(ui.ctx(), id.clone());
                copied = Some(id.clone());
            }
        }
    });
    if let Some(id) = selected {
        app.draft.project_input = id.clone();
        sync_input(app);
        app.remember_project(&id);
        fetch = true;
    }
    if let Some(id) = copied {
        app.remember_project(&id);
        app.toasts.success("已复制项目编号");
    }
    if fetch && can_fetch(app) {
        app.fetch_project();
    }

    match &app.draft.parsed {
        ParseResult::Empty => {}
        ParseResult::Id { id, .. } => {
            widgets::small(ui, format!("识别为项目 ID {id}"), p.text_3);
        }
        ParseResult::ShortLink => {
            widgets::small(ui, "暂不支持 b23.tv 短链接，请在浏览器打开后复制完整地址", p.danger);
        }
        ParseResult::Invalid => {
            widgets::small(
                ui,
                "没有识别到项目 ID，请粘贴 show.bilibili.com 或 mall.bilibili.com 的链接，或直接输入数字 ID",
                p.danger,
            );
        }
    }
}

fn project_status(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let fetchable = can_fetch(app);
    let referer = match &app.draft.parsed {
        ParseResult::Id { referer, .. } => Some(referer.clone()),
        _ => None,
    };
    // fetch_project 在账号没有 cookie_manager 时直接写入 Failed，这种情况只显示原因，不加“获取失败”前缀
    let no_client = app.draft.account_uid.and_then(|uid| app.account(uid)).map_or(false, |a| a.cookie_manager.is_none());
    let mut retry = false;
    match &app.draft.project {
        Remote::Idle => {}
        Remote::Loading { .. } => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let w = ui.available_width();
                widgets::skeleton(ui, w, 20.0);
                widgets::skeleton(ui, w * 0.6, 14.0);
                widgets::skeleton(ui, w * 0.4, 14.0);
            });
        }
        Remote::Failed { message, .. } => {
            if no_client {
                widgets::small(ui, message.as_str(), p.danger);
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.add(Label::new(RichText::new(format!("获取项目信息失败：{message}")).color(p.danger)).wrap(true));
                    retry = button_ex(ui, "重试", Kind::Ghost, Size::Sm, false, fetchable).clicked();
                });
            }
        }
        Remote::Ready { value, .. } => {
            if project_card(ui, p, value) {
                if let Some(url) = &referer {
                    widgets::open_url(ui.ctx(), url);
                }
            }
        }
    }
    if retry {
        app.fetch_project();
    }
}

/// 返回是否点了“在浏览器打开”
fn project_card(ui: &mut Ui, p: &Palette, project: &TicketInfo) -> bool {
    let mut open = false;
    egui::Frame::none().fill(p.sunken).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        let button_w = text_width(ui, "在浏览器打开", 13.0) + 20.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - button_w - 8.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                let name = project.name.as_deref().unwrap_or("未知项目");
                ui.add(Label::new(RichText::new(name).font(bold(16.0)).color(p.text)).wrap(true));
                let info = base_info(project);
                if !info.is_empty() {
                    ui.add(Label::new(RichText::new(info).size(12.0).color(p.text_2)).wrap(true));
                }
            });
            open = button_ex(ui, "在浏览器打开", Kind::Ghost, Size::Sm, false, true).clicked();
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !project.sale_flag.is_empty() {
                widgets::pill(ui, project.sale_flag.clone(), Tone::Neutral);
            }
            widgets::pill(ui, bind_label(project.id_bind), Tone::Primary);
            if project.hot_project == Some(true) {
                widgets::pill(ui, "热门", Tone::Warning);
            }
        });
        if let Some(ts) = project.sale_begin.filter(|t| *t > 0) {
            widgets::small(ui, format!("开售 {}", fmt_time(ts, "%Y-%m-%d %H:%M:%S")), p.text_2);
        }
    });
    open
}

fn mode_card(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    widgets::card(ui, |ui| {
        widgets::section_title(ui, "抢票方式", |_| {});
        let mut mode = app.draft.mode;
        widgets::segmented(ui, &mut mode, &[(0u8, MODE_NAMES[0]), (1, MODE_NAMES[1]), (2, MODE_NAMES[2])]);
        app.draft.mode = mode;

        let ticket_clickable = selected_ticket(app).map(|t| t.clickable == Some(true));
        match mode {
            0 => {
                ui.label(RichText::new("到开售时间自动开始下单，适合还没开售的项目。").color(p.text_2));
                if ticket_clickable == Some(true) {
                    widgets::small(ui, "该票种已在售，开始后会立即下单", p.text_3);
                }
            }
            1 => {
                ui.label(RichText::new("立即反复下单，只用于已开售的票种。").color(p.text_2));
                if ticket_clickable == Some(false) {
                    let switch = in_row(ui, |ui| {
                        widgets::callout(
                            ui,
                            "该票种现在不可售。直接下单会持续提交不可售订单，可能导致账号被限制。",
                            Tone::Warning,
                            Some("改用定时抢票"),
                        )
                    });
                    if switch {
                        app.draft.mode = 0;
                    }
                }
            }
            _ => {
                ui.label(RichText::new("持续检查全部场次和票种，有余票立即下单，只支持实名票。").color(p.text_2));
                if project_bind(app) == Some(Some(0)) {
                    in_row(ui, |ui| widgets::callout(ui, "该项目为非实名票，捡漏回流暂不支持", Tone::Danger, None));
                }
            }
        }
        if mode != 2 && app.draft.project.value().map_or(false, nothing_on_sale) {
            widgets::small(ui, "当前没有可售票，可以试试捡漏回流", p.text_3);
        }
    });
}

fn ticket_card(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let mut new_screen = None;
    let mut new_ticket = None;
    widgets::card(ui, |ui| {
        widgets::section_title(ui, "场次与票种", |_| {});
        let Some(project) = app.draft.project.value() else {
            widgets::small(ui, "获取项目后在这里选择场次和票种", p.text_3);
            return;
        };
        ui.horizontal_wrapped(|ui| {
            for (i, screen) in project.screen_list.iter().enumerate() {
                if screen_chip(ui, p, screen, app.draft.screen_idx == Some(i)).clicked() {
                    new_screen = Some(i);
                }
            }
        });
        let Some(screen) = selected_screen(app) else { return };
        let direct = app.draft.mode == 1;
        egui::Frame::none()
            .stroke(Stroke::new(1.0, p.border))
            .rounding(8.0)
            .inner_margin(1.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let n = screen.ticket_list.len();
                for (i, t) in screen.ticket_list.iter().enumerate() {
                    if i > 0 {
                        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().rect_filled(line, 0.0, p.border);
                    }
                    let id = t.id.unwrap_or(0) as i64;
                    let enabled = direct || timed_selectable(t);
                    let selected = app.draft.ticket_id == Some(id);
                    let rounding = Rounding {
                        nw: if i == 0 { 7.0 } else { 0.0 },
                        ne: if i == 0 { 7.0 } else { 0.0 },
                        sw: if i + 1 == n { 7.0 } else { 0.0 },
                        se: if i + 1 == n { 7.0 } else { 0.0 },
                    };
                    let resp = ticket_row(ui, p, t, selected, enabled, rounding);
                    if !enabled {
                        resp.on_hover_text("该票种已售罄或不可售");
                    } else if resp.clicked() {
                        new_ticket = Some(id);
                    }
                }
            });
    });
    if let Some(i) = new_screen {
        if app.draft.screen_idx != Some(i) {
            app.draft.screen_idx = Some(i);
            app.draft.ticket_id = None;
        }
    }
    if let Some(id) = new_ticket {
        app.draft.ticket_id = Some(id);
    }
}

/// 场次选择块：两行（场次名、售卖状态），选中 primary 描边 + primary_soft 底
fn screen_chip(ui: &mut Ui, p: &Palette, screen: &ScreenInfo, selected: bool) -> Response {
    let name = screen.name.clone().unwrap_or_else(|| "未知场次".into());
    let (name_color, sub_color) = if screen.clickable { (p.text, p.text_2) } else { (p.text_3, p.text_3) };
    let g1 = ui.painter().layout_no_wrap(name, FontId::proportional(14.0), name_color);
    let g2 = ui.painter().layout_no_wrap(screen.sale_flag.display_name.clone(), FontId::proportional(12.0), sub_color);
    let (s1, s2) = (g1.size(), g2.size());
    let size = vec2(s1.x.max(s2.x) + 24.0, s1.y + s2.y + 2.0 + 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (fill, stroke) = if selected {
            (p.primary_soft, p.primary)
        } else if resp.hovered() {
            (p.sunken, p.border_strong)
        } else {
            (p.surface, p.border)
        };
        painter.rect_filled(rect, 8.0, fill);
        painter.rect_stroke(rect, 8.0, Stroke::new(1.0, stroke));
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0, p.primary));
        }
        painter.galley(rect.min + vec2(12.0, 6.0), g1);
        painter.galley(rect.min + vec2(12.0, 6.0 + s1.y + 2.0), g2);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// 票种行：单选圈 · 票种名 · 状态标签 · 价格（右对齐）
fn ticket_row(ui: &mut Ui, p: &Palette, t: &ScreenTicketInfo, selected: bool, enabled: bool, rounding: Rounding) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, rounding, p.primary_soft);
        } else if enabled && resp.hovered() {
            painter.rect_filled(rect, rounding, p.sunken);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.shrink(1.0), rounding, Stroke::new(2.0, p.primary));
        }
        let cy = rect.center().y;
        let text_color = if enabled { p.text } else { p.text_3 };
        let radio = pos2(rect.left() + 22.0, cy);
        if selected {
            painter.circle_stroke(radio, 7.0, Stroke::new(1.5, p.primary));
            painter.circle_filled(radio, 4.0, p.primary);
        } else {
            painter.circle_filled(radio, 7.0, p.surface);
            painter.circle_stroke(radio, 7.0, Stroke::new(1.0, if enabled { p.border_strong } else { p.border }));
        }
        let price = painter.text(
            pos2(rect.right() - 16.0, cy),
            Align2::RIGHT_CENTER,
            widgets::money(t.price.unwrap_or(0) as i64),
            FontId::monospace(13.0),
            text_color,
        );
        let (status, tone) = ticket_status(t);
        let pill_left = paint_pill(painter, pos2(price.left() - 12.0, cy), status, tone, p);
        let desc_rect = Rect::from_min_max(pos2(rect.left() + 38.0, rect.top()), pos2(pill_left - 8.0, rect.bottom()));
        painter.with_clip_rect(desc_rect).text(
            pos2(desc_rect.left(), cy),
            Align2::LEFT_CENTER,
            t.desc.as_deref().unwrap_or("未知票种"),
            FontId::proportional(14.0),
            text_color,
        );
    }
    if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
}

/// 在 painter 上画状态标签，right_center 为右缘中点，返回标签左缘 x
fn paint_pill(painter: &Painter, right_center: Pos2, text: String, tone: Tone, p: &Palette) -> f32 {
    let (fg, bg) = p.tone(tone);
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), fg);
    let size = vec2(galley.size().x + 16.0, 22.0);
    let rect = Rect::from_min_size(pos2(right_center.x - size.x, right_center.y - 11.0), size);
    painter.rect_filled(rect, 11.0, bg);
    painter.galley(rect.center() - galley.size() / 2.0, galley);
    rect.left()
}

/// 捡漏模式：跳过关键词与只读的场次票种清单
fn scope_card(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let mut remove = None;
    let mut added = false;
    widgets::card(ui, |ui| {
        widgets::section_title(ui, "监控范围", |_| {});
        let resp = widgets::text_field(
            ui,
            &mut app.draft.skip_input,
            FieldOpts { label: Some("跳过关键词"), hint: "输入关键词后回车", id: Some("new_task_skip"), ..Default::default() },
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let words: Vec<String> = app.draft.skip_input.split_whitespace().map(String::from).collect();
            for w in words {
                if !app.draft.skip_words.contains(&w) {
                    app.draft.skip_words.push(w);
                    added = true;
                }
            }
            app.draft.skip_input.clear();
            resp.request_focus();
        }
        if !app.draft.skip_words.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (i, w) in app.draft.skip_words.iter().enumerate() {
                    if keyword_chip(ui, p, w) {
                        remove = Some(i);
                    }
                }
            });
        }
        widgets::small(ui, "含任一关键词的场次或票种不会下单", p.text_3);

        match &app.draft.project {
            Remote::Ready { value, .. } => scope_list(ui, p, value, &app.draft.skip_words),
            Remote::Failed { .. } => {
                widgets::small(ui, "项目信息获取失败，仍可只用项目 ID 开始捡漏", p.text_3);
            }
            _ => {}
        }
    });
    if let Some(i) = remove {
        app.draft.skip_words.remove(i);
    }
    if added || remove.is_some() {
        app.save_skip_words();
    }
}

/// 关键词标签（Primary）+ “×”，返回是否点了 ×
fn keyword_chip(ui: &mut Ui, p: &Palette, word: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(word.to_owned(), FontId::proportional(12.0), p.primary);
    let text_size = galley.size();
    let (rect, _) = ui.allocate_exact_size(vec2(text_size.x + 10.0 + 18.0, 22.0), Sense::hover());
    let close_rect = Rect::from_min_max(pos2(rect.right() - 20.0, rect.top()), rect.max);
    let close = ui.interact(close_rect, ui.id().with(("skip_word", word)), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 11.0, p.primary_soft);
        painter.galley(pos2(rect.left() + 10.0, rect.center().y - text_size.y / 2.0), galley);
        let color = if close.hovered() { p.primary_hover } else { p.primary };
        painter.text(close_rect.center(), Align2::CENTER_CENTER, "×", FontId::proportional(14.0), color);
    }
    close.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn scope_list(ui: &mut Ui, p: &Palette, project: &TicketInfo, words: &[String]) {
    let entry = |ui: &mut Ui, name: &str, indent: f32, size: f32| {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            let skip = skipped(name, words);
            let mut text = RichText::new(name).size(size);
            text = if skip { text.color(p.text_3).strikethrough() } else { text.color(if indent > 0.0 { p.text_2 } else { p.text }) };
            ui.label(text);
            if skip {
                widgets::pill(ui, "跳过", Tone::Neutral);
            }
        });
    };
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 28.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for screen in &project.screen_list {
            entry(ui, screen.name.as_deref().unwrap_or("未知场次"), 0.0, 14.0);
            for t in &screen.ticket_list {
                entry(ui, t.desc.as_deref().unwrap_or("未知票种"), 16.0, 13.0);
            }
        }
    });
}

fn buyer_card(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let mode = buyer_mode(app);
    let uid = app.draft.account_uid;
    let chosen_n = app.draft.selected_buyers.len();
    let lists = matches!(mode, BuyerMode::Single | BuyerMode::Multi);
    let cooldown = uid.and_then(|u| app.buyers.get(&u)).map_or(0, |r| r.cooldown_left(5));
    let mut refresh = false;
    let mut add = false;
    let mut toggle = None;

    widgets::card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("购票人").font(bold(16.0)).color(p.text));
            if mode == BuyerMode::Multi {
                widgets::small(ui, format!("共 {chosen_n} 张"), p.text_3);
            }
            if uid.is_some() && lists {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    add = button_ex(ui, "+ 添加购票人", Kind::Secondary, Size::Sm, false, true).clicked();
                    let resp = button_ex(ui, "刷新", Kind::Ghost, Size::Sm, false, cooldown == 0);
                    refresh = resp.clicked();
                    if cooldown > 0 {
                        resp.on_hover_text(format!("{cooldown} 秒后可刷新"));
                    }
                });
            }
        });

        let Some(uid) = uid else {
            widgets::small(ui, "登录账号后选择购票人", p.text_3);
            return;
        };
        match mode {
            BuyerMode::Pending => {
                widgets::small(ui, "获取项目后在这里选择购票人", p.text_3);
            }
            BuyerMode::Unknown => {
                widgets::small(ui, "该项目实名类型未知，暂不支持选择购票人", p.warning);
            }
            BuyerMode::NoBind => no_bind_fields(app, ui),
            BuyerMode::Single | BuyerMode::Multi => {
                let check = if mode == BuyerMode::Single { Check::Single } else { Check::Multi };
                match app.buyers.get(&uid) {
                    None | Some(Remote::Idle) | Some(Remote::Loading { .. }) => {
                        ui.horizontal(|ui| {
                            for _ in 0..3 {
                                widgets::skeleton(ui, 220.0, 72.0);
                            }
                        });
                    }
                    Some(Remote::Failed { message, .. }) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(Label::new(RichText::new(format!("获取购票人失败：{message}")).color(p.danger)).wrap(true));
                            refresh |= button_ex(ui, "重试", Kind::Ghost, Size::Sm, false, cooldown == 0).clicked();
                        });
                    }
                    Some(Remote::Ready { value, .. }) => {
                        if value.from_cache {
                            widgets::small(ui, "网络获取失败，正在使用本地缓存的购票人列表", p.warning);
                        }
                        if value.list.is_empty() {
                            let resp = widgets::empty_state(ui, "这个账号还没有购票人", "实名票需要先添加购票人", Some("添加购票人"));
                            add |= resp.map_or(false, |r| r.clicked());
                        } else {
                            toggle = buyer_grid(ui, p, &value.list, &app.draft.selected_buyers, check);
                        }
                    }
                }
                if mode == BuyerMode::Single {
                    count_row(ui, &mut app.draft.count);
                }
            }
        }
    });

    let Some(uid) = uid else { return };
    if refresh {
        app.request_buyers(uid);
    }
    if add {
        app.open_add_buyer(uid);
    }
    if let Some(id) = toggle {
        let sel = &mut app.draft.selected_buyers;
        if mode == BuyerMode::Single {
            *sel = vec![id];
        } else if let Some(pos) = sel.iter().position(|x| *x == id) {
            sel.remove(pos);
        } else if sel.len() >= MAX_BUYERS {
            app.toasts.warn("最多选择 10 位购票人");
        } else {
            sel.push(id);
        }
    }
}

/// 购票人卡片网格，列数 = floor(可用宽 / 220)；返回被点击的购票人 ID
fn buyer_grid(ui: &mut Ui, p: &Palette, list: &[BuyerInfo], selected: &[i64], check: Check) -> Option<i64> {
    let gap = 8.0;
    let avail = ui.available_width();
    let cols = ((avail / 220.0).floor() as usize).max(1);
    let width = (avail - gap * (cols - 1) as f32) / cols as f32;
    let mut clicked = None;
    for row in list.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for b in row {
                let resp = widgets::select_card(ui, selected.contains(&b.id), check, width, |ui| buyer_body(ui, p, b));
                if resp.clicked() {
                    clicked = Some(b.id);
                }
            }
        });
    }
    clicked
}

fn buyer_body(ui: &mut Ui, p: &Palette, b: &BuyerInfo) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.spacing_mut().interact_size.y = 22.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(Label::new(RichText::new(&b.name).color(p.text)).truncate(true));
        if b.is_default == 1 {
            widgets::pill(ui, "默认", Tone::Neutral);
        }
    });
    widgets::small(ui, format!("{} · {}", id_type_label(b.id_type), widgets::mask_id(&b.personal_id)), p.text_3);
    widgets::small(ui, widgets::mask_phone(&b.tel), p.text_3);
}

fn count_row(ui: &mut Ui, count: &mut i32) {
    ui.horizontal(|ui| {
        ui.label("购买数量");
        widgets::stepper(ui, count, 1, 10);
    });
}

/// 非实名：数量 + 联系人姓名、手机号
fn no_bind_fields(app: &mut Myapp, ui: &mut Ui) {
    count_row(ui, &mut app.draft.count);
    let touched_id = egui::Id::new("new_task_tel_touched");
    let touched = ui.data(|d| d.get_temp::<bool>(touched_id)).unwrap_or(false);
    let error = (touched && !tel_valid(&app.draft.contact_tel)).then(|| "请输入 11 位手机号".to_string());
    ui.horizontal_top(|ui| {
        widgets::text_field(
            ui,
            &mut app.draft.contact_name,
            FieldOpts { label: Some("联系人姓名"), hint: "姓名", width: Some(220.0), id: Some("new_task_contact_name"), ..Default::default() },
        );
        let resp = widgets::text_field(
            ui,
            &mut app.draft.contact_tel,
            FieldOpts {
                label: Some("手机号"),
                hint: "11 位手机号",
                width: Some(220.0),
                filter: Filter::Digits,
                error,
                id: Some("new_task_contact_tel"),
                ..Default::default()
            },
        );
        if resp.lost_focus() {
            ui.data_mut(|d| d.insert_temp(touched_id, true));
        }
    });
    app.draft.contact_tel.truncate(11);
}

// ---------------------------------------------------------------- 摘要与提交

struct Summary {
    rows: Vec<(&'static str, String)>,
    /// 距开售秒数（模式 0 且有开售时间）
    countdown: Option<i64>,
    /// (张数, 金额分)；模式 2 为 None，未选票种时金额为 None
    total: Option<(i32, Option<i64>)>,
    ticket_text: String,
    people: usize,
    missing: Vec<&'static str>,
    duplicate: Option<u64>,
}

fn summary(app: &Myapp) -> Summary {
    let d = &app.draft;
    let dash = || "—".to_string();
    let mode = buyer_mode(app);
    let project = d.project.value();
    let screen = selected_screen(app);
    let ticket = selected_ticket(app);
    let chosen = chosen_buyers(app);
    let all_text = if d.skip_words.is_empty() {
        "全部".to_string()
    } else {
        format!("全部（跳过 {} 个关键词）", d.skip_words.len())
    };
    let n = if mode == BuyerMode::Multi { chosen.len() as i32 } else { d.count };

    let account = d.account_uid.and_then(|uid| app.account(uid)).map(|a| a.name.clone()).unwrap_or_else(dash);
    let project_text = match (project, &d.parsed) {
        (Some(p), _) => p.name.clone().unwrap_or_else(dash),
        (None, ParseResult::Id { id, .. }) => format!("ID {id}"),
        _ => dash(),
    };
    let (screen_text, ticket_text) = if d.mode == 2 {
        (all_text.clone(), all_text)
    } else {
        (
            screen.and_then(|s| s.name.clone()).unwrap_or_else(dash),
            ticket.and_then(|t| t.desc.clone()).unwrap_or_else(dash),
        )
    };
    let count_text = if mode == BuyerMode::Multi && chosen.is_empty() { dash() } else { format!("{n} 张") };
    let buyers_text = match mode {
        BuyerMode::Single | BuyerMode::Multi if !chosen.is_empty() => {
            chosen.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join("、")
        }
        BuyerMode::NoBind if !d.contact_name.trim().is_empty() => d.contact_name.trim().to_string(),
        _ => dash(),
    };

    let countdown = if d.mode == 0 {
        project.and_then(|p| {
            let screen_start = screen.map(|s| s.sale_start as i64).filter(|t| *t > 0);
            screen_start.or(p.sale_begin.filter(|t| *t > 0)).map(|t| t - now_secs())
        })
    } else {
        None
    };
    let total = (d.mode != 2).then(|| (n, ticket.map(|t| t.price.unwrap_or(0) as i64 * n as i64)));

    Summary {
        rows: vec![
            ("账号", account),
            ("项目", project_text),
            ("场次", screen_text),
            ("票种", ticket_text),
            ("数量", count_text),
            ("购票人", buyers_text),
            ("方式", MODE_NAMES[d.mode as usize].to_string()),
        ],
        countdown,
        total,
        ticket_text: if d.mode == 2 {
            "全部票种".into()
        } else {
            ticket.and_then(|t| t.desc.clone()).unwrap_or_else(|| "未选票种".into())
        },
        people: match mode {
            BuyerMode::Single | BuyerMode::Multi => chosen.len(),
            _ => d.count as usize,
        },
        missing: missing(app),
        duplicate: duplicate_seq(app),
    }
}

fn start_label(app: &Myapp, s: &Summary) -> &'static str {
    if s.duplicate.is_some() {
        return "再开一个相同任务";
    }
    match app.draft.mode {
        0 => "开始定时抢票",
        1 => "立即开始下单",
        _ => "开始捡漏",
    }
}

fn summary_panel(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let s = summary(app);
    let mut start = false;
    let mut clear = false;
    egui::ScrollArea::vertical()
        .id_source("new_task_summary")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::none()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .rounding(10.0)
                .inner_margin(18.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 14.0;
                    ui.label(RichText::new("任务摘要").font(bold(16.0)).color(p.text));

                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (k, v) in &s.rows {
                            kv_row(ui, p, k, v);
                        }
                    });

                    if let Some(secs) = s.countdown {
                        egui::Frame::none().fill(p.sunken).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 4.0;
                            widgets::small(ui, "距开售", p.text_3);
                            let (text, color) = if secs <= 0 {
                                ("已开售".to_string(), p.text)
                            } else {
                                (widgets::countdown_text(secs), if secs <= 10 { p.primary } else { p.text })
                            };
                            ui.label(RichText::new(text).font(FontId::monospace(28.0)).color(color));
                        });
                    }

                    if let Some((n, fen)) = s.total {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("合计 {n} 张")).color(p.text_2));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let amount = fen.map(widgets::money).unwrap_or_else(|| "—".into());
                                ui.label(RichText::new(amount).font(bold(20.0)).color(p.text));
                            });
                        });
                    }

                    if !s.missing.is_empty() {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for m in &s.missing {
                                widgets::small(ui, format!("· {m}"), p.text_2);
                            }
                        });
                    }
                    if let Some(seq) = s.duplicate {
                        widgets::small(ui, format!("已有相同任务在进行（No.{seq:04}）"), p.warning);
                    }

                    let label = start_label(app, &s);
                    start = button_ex(ui, label, Kind::Primary, Size::Xl, false, s.missing.is_empty()).clicked();
                    ui.vertical_centered(|ui| clear = clear_link(ui, p).clicked());
                });
        });
    if start {
        submit(app);
    }
    if clear {
        clear_selection(app);
    }
}

fn kv_row(ui: &mut Ui, p: &Palette, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(vec2(56.0, 18.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(56.0);
            ui.label(RichText::new(key).color(p.text_3));
        });
        let color = if value == "—" { p.text_3 } else { p.text };
        ui.add(Label::new(RichText::new(value).color(color)).wrap(true));
    });
}

fn clear_link(ui: &mut Ui, p: &Palette) -> Response {
    let resp = ui.add(Label::new(RichText::new("清空选择").size(12.0).color(p.text_3)).sense(Sense::click()));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// 窄宽底栏：左侧一行摘要，右侧主按钮
fn summary_bar(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let s = summary(app);
    let mut start = false;
    let mut clear = false;
    let left = match s.total {
        Some((_, Some(fen))) => format!("{} · {} · {} 人", s.ticket_text, widgets::money(fen), s.people),
        Some((_, None)) => format!("{} · — · {} 人", s.ticket_text, s.people),
        None => format!("{} · {} 人", s.ticket_text, s.people),
    };
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = start_label(app, &s);
            let enabled = s.missing.is_empty();
            let mut resp = ui
                .allocate_ui_with_layout(vec2(180.0, 40.0), Layout::top_down(Align::Min), |ui| {
                    button_ex(ui, label, Kind::Primary, Size::Xl, false, enabled)
                })
                .inner;
            if let Some(first) = s.missing.first() {
                resp = resp.on_hover_text(*first);
            }
            start = resp.clicked();
            clear = clear_link(ui, p).clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(left).color(p.text)).truncate(true));
                if let Some(seq) = s.duplicate {
                    widgets::small(ui, format!("已有相同任务（No.{seq:04}）"), p.warning);
                }
            });
        });
    });
    if start {
        submit(app);
    }
    if clear {
        clear_selection(app);
    }
}

/// 清空项目、场次、票种、购票人选择，数量回 1；保留账号、模式、关键词
fn clear_selection(app: &mut Myapp) {
    let d = &mut app.draft;
    d.project_input.clear();
    d.parsed = ParseResult::Empty;
    d.last_paste_len = 0;
    d.project = Remote::Idle;
    d.project_for = None;
    d.screen_idx = None;
    d.ticket_id = None;
    d.selected_buyers.clear();
    d.count = 1;
}

/// 取值与旧 confirm_ticket.rs（模式 0/1）、confirm_ticket2.rs（模式 2）一致，见规格 6.2
fn submit(app: &mut Myapp) {
    let Some(uid) = app.draft.account_uid else { return };
    let Some(account) = app.account(uid).cloned() else { return };
    let Some(cookie_manager) = account.cookie_manager.clone() else { return };
    let ParseResult::Id { id: parsed_id, referer } = app.draft.parsed.clone() else { return };
    let mode = app.draft.mode;
    let chosen = chosen_buyers(app);
    let mut bt = BilibiliTicket::new(
        &mode,
        &app.default_ua,
        &app.custom_config,
        &account,
        &app.push_config,
        &app.status_delay,
        &parsed_id,
        &referer,
    );
    let mut contact = None;

    let request = if mode == 2 {
        bt.project_info = app.draft.project.value().cloned();
        bt.id_bind = 1;
        GrabTicketRequest {
            task_id: "".to_string(),
            uid,
            project_id: parsed_id.clone(),
            screen_id: "".to_string(),
            ticket_id: "".to_string(),
            count: chosen.len() as i16,
            buyer_info: chosen,
            is_hot: true,
            grab_mode: 2,
            status: TaskStatus::Pending,
            start_time: None,
            cookie_manager,
            biliticket: bt.clone(),
            local_captcha: app.local_captcha.clone(),
            skip_words: (!app.draft.skip_words.is_empty()).then(|| app.draft.skip_words.clone()),
        }
    } else {
        let Some(project) = app.draft.project.value().cloned() else { return };
        let Some(screen) = selected_screen(app).cloned() else { return };
        let Some(ticket) = selected_ticket(app).cloned() else { return };
        let Some(id_bind) = project.id_bind else { return };
        bt.project_info = Some(project.clone());
        bt.id_bind = id_bind;
        bt.screen_id = screen.id.unwrap_or(0).to_string();
        bt.select_ticket_id = Some(ticket.id.unwrap_or(0).to_string());
        let (buyer_info, count_final) = match id_bind {
            0 => {
                let name = app.draft.contact_name.clone();
                let tel = app.draft.contact_tel.clone();
                bt.no_bind_buyer_info = Some(NoBindBuyerInfo { name: name.clone(), tel: tel.clone(), uid });
                contact = Some(Contact { name, tel });
                (Vec::new(), app.draft.count)
            }
            1 => {
                let Some(buyer) = chosen.first().cloned() else { return };
                bt.buyer_info = Some(vec![buyer.clone()]);
                (vec![buyer], app.draft.count)
            }
            2 => {
                let n = chosen.len() as i32;
                (chosen, n)
            }
            _ => return,
        };
        bt.count = Some(count_final);
        GrabTicketRequest {
            task_id: "".to_string(),
            uid,
            project_id: project.id.unwrap_or(0).to_string(),
            screen_id: screen.id.unwrap_or(0).to_string(),
            ticket_id: ticket.id.unwrap_or(0).to_string(),
            count: count_final as i16,
            buyer_info,
            cookie_manager,
            biliticket: bt.clone(),
            grab_mode: mode,
            status: TaskStatus::Pending,
            start_time: None,
            is_hot: project.hot_project.unwrap_or(false),
            local_captcha: app.local_captcha.clone(),
            skip_words: None,
        }
    };

    let key = (uid, request.project_id.clone(), request.ticket_id.clone());
    match app.task_manager.submit_task(TaskRequest::GrabTicketRequest(request)) {
        Ok(task_id) => {
            log::info!("提交抢票任务成功，任务ID: {}", task_id);
            app.remember_project(&parsed_id);
            app.board.submitted.push((task_id, key.0, key.1, key.2));
            if let Some(contact) = contact {
                app.contacts.insert(uid.to_string(), contact);
                app.save_contacts();
            }
            app.toasts.push(Tone::Success, "任务已开始", None, Some(("查看任务".into(), ToastAction::GoTasks)));
        }
        Err(e) => {
            log::error!("提交抢票任务失败: {}", e);
            app.toasts.push(Tone::Danger, format!("提交任务失败：{e}"), None, None);
        }
    }
}
