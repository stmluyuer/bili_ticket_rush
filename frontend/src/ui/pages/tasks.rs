use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use common::taskmanager::{GrabPhase, RunningTaskInfo, TaskOutcome, TaskStatus};
use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Align2, Color32, CursorIcon, FontId, Label, Layout,
    Margin, Rect, Response, RichText, Sense, Shape, Stroke, TextFormat, Ui,
};

use crate::app::{Dialog, Myapp, Page};
use crate::state::logs::{display_label, Level, LevelFilter, LogLine};
use crate::state::tasks::{TaskBoard, TasksTab};
use crate::ui::theme::{bold, palette, Palette, Tone};
use crate::ui::widgets::{self, FieldOpts, Kind, Size};

/// 票根右侧存根宽度
const STUB_W: f32 = 216.0;
const CARD_MIN_H: f32 = 96.0;
/// 日志行高；show_rows 要求每行等高
const LOG_ROW_H: f32 = 20.0;

/// 列表与日志面板里收集的操作，渲染结束后统一处理，避免渲染时可变借用 app
enum Action {
    OpenLog { seq: u64, account: String },
    AskStop(String),
    ConfirmStop { task_id: String, seq: u64 },
    CancelStop,
    Pay(String),
    GoNewTask,
}

enum LogAction { ToggleFollow, Copy, OpenFolder, Clear }

pub fn render(app: &mut Myapp, ui: &mut Ui) {
    let p = palette(ui.ctx());

    egui::Frame::none()
        .inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 12.0 })
        .show(ui, |ui| {
            header(app, ui);
            if let (Some(text), false) = (app.announce3.clone(), app.notice_dismissed[1]) {
                ui.add_space(4.0);
                if widgets::notice_bar(ui, &text, Tone::Info) {
                    app.notice_dismissed[1] = true;
                }
            }
        });

    // 底部面板必须在列表之前声明，列表才会占用面板以上的剩余空间
    if app.log_panel_open {
        log_panel(app, ui, &p);
    }

    let action = egui::ScrollArea::vertical()
        .id_source("task_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(Margin { left: 24.0, right: 24.0, top: 4.0, bottom: 24.0 })
                .show(ui, |ui| task_list(app.tasks_tab, &app.board, ui, &p))
                .inner
        })
        .inner;

    if let Some(action) = action {
        apply_action(app, action);
    }
}

fn header(app: &mut Myapp, ui: &mut Ui) {
    let run_label = format!("进行中 ({})", app.board.running.len());
    let fin_label = format!("已结束 ({})", app.board.finished.len());
    widgets::page_title(ui, "任务", |ui| {
        // 从右到左：日志、页签、清除已结束（页签位置不随切换移动）
        if toggle_button(ui, "日志", app.log_panel_open, true).clicked() {
            app.log_panel_open = !app.log_panel_open;
        }
        widgets::segmented(
            ui,
            &mut app.tasks_tab,
            &[(TasksTab::Running, run_label.as_str()), (TasksTab::Finished, fin_label.as_str())],
        );
        if app.tasks_tab == TasksTab::Finished {
            let enabled = !app.board.finished.is_empty();
            if widgets::button_ex(ui, "清除已结束", Kind::Ghost, Size::Sm, false, enabled).clicked() {
                app.board.finished.clear();
            }
        }
    });
}

fn apply_action(app: &mut Myapp, action: Action) {
    match action {
        Action::OpenLog { seq, account } => {
            app.log_filter.label = Some(format!("#{} {}", seq, account));
            app.log_panel_open = true;
        }
        Action::AskStop(task_id) => app.board.stop_confirm = Some((task_id, Instant::now())),
        Action::CancelStop => app.board.stop_confirm = None,
        Action::ConfirmStop { task_id, seq } => {
            app.board.stop_confirm = None;
            match app.task_manager.cancel_task(&task_id) {
                Ok(()) => {
                    log::info!("已请求取消任务: {}", task_id);
                    app.toasts.neutral(format!("已停止 No.{:04}", seq));
                }
                Err(e) => {
                    log::error!("取消任务失败: {}", e);
                    app.toasts.danger(format!("停止失败：{}", e));
                }
            }
        }
        Action::Pay(task_id) => {
            if let Some(index) = app.board.payments.iter().position(|pm| pm.task_id == task_id) {
                app.board.payment_index = index;
                app.close_dialog();
                app.dialog = Dialog::Payment;
            }
        }
        Action::GoNewTask => app.page = Page::NewTask,
    }
}

// ---------- 票根卡片 ----------

fn task_list(tab: TasksTab, board: &TaskBoard, ui: &mut Ui, p: &Palette) -> Option<Action> {
    ui.spacing_mut().item_spacing.y = 12.0;
    let mut action = None;
    match tab {
        TasksTab::Running => {
            if board.running.is_empty() {
                let resp = widgets::empty_state(ui, "没有进行中的任务", "在抢票页选好票种后开始", Some("去抢票"));
                if resp.map_or(false, |r| r.clicked()) {
                    action = Some(Action::GoNewTask);
                }
            }
            for info in &board.running {
                let confirming = board.stop_confirm.as_ref().map_or(false, |(id, _)| *id == info.task_id);
                if let Some(a) = ticket_card(ui, p, info, None, confirming, None) {
                    action = Some(a);
                }
            }
        }
        TasksTab::Finished => {
            if board.finished.is_empty() {
                widgets::empty_state(ui, "还没有结束的任务", "本次运行期间结束的任务会显示在这里", None);
            }
            for finished in &board.finished {
                let amount = board
                    .payments
                    .iter()
                    .find(|pm| pm.task_id == finished.info.task_id)
                    .map(|pm| pm.amount_fen);
                if let Some(a) = ticket_card(ui, p, &finished.info, Some(&finished.outcome), false, amount) {
                    action = Some(a);
                }
            }
        }
    }
    action
}

fn is_cancelling(info: &RunningTaskInfo) -> bool {
    matches!(&info.status, TaskStatus::Failed(msg) if msg == "取消中")
}

/// 阶段 pill 的语气与文字，左缘竖条同色
fn stage(info: &RunningTaskInfo, outcome: Option<&TaskOutcome>) -> (Tone, &'static str) {
    match outcome {
        Some(TaskOutcome::Succeeded) => (Tone::Success, "已抢到"),
        Some(TaskOutcome::Stopped) => (Tone::Neutral, "已停止"),
        Some(TaskOutcome::Ended(_)) => (Tone::Danger, "已结束"),
        None if is_cancelling(info) => (Tone::Neutral, "停止中"),
        None => match info.phase {
            GrabPhase::Preparing => (Tone::Neutral, "准备中"),
            GrabPhase::WaitingSale => (Tone::Neutral, "等待开售"),
            GrabPhase::Grabbing => (Tone::Info, "抢票中"),
            GrabPhase::Captcha => (Tone::Warning, "验证码"),
            GrabPhase::Monitoring => (Tone::Info, "监控中"),
        },
    }
}

fn mode_label(mode: u8) -> &'static str {
    match mode {
        0 => "定时",
        1 => "直接",
        2 => "捡漏",
        _ => "未知",
    }
}

fn mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn ago(at: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(at).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 { format!("{} 秒前", secs) } else { format!("{} 分钟前", secs / 60) }
}

/// 左区 + 右存根；amount 为该任务待付款金额（分），只对已抢到的卡有意义
fn ticket_card(
    ui: &mut Ui,
    p: &Palette,
    info: &RunningTaskInfo,
    outcome: Option<&TaskOutcome>,
    confirming: bool,
    amount: Option<i64>,
) -> Option<Action> {
    let width = ui.available_width();
    let left_w = (width - STUB_W).max(160.0);
    let (tone, stage_text) = stage(info, outcome);
    let bg = ui.painter().add(Shape::Noop);
    let mut action = None;

    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.set_min_height(CARD_MIN_H);
        ui.vertical(|ui| {
            ui.set_width(left_w);
            egui::Frame::none()
                .inner_margin(Margin { left: 18.0, right: 16.0, top: 12.0, bottom: 12.0 })
                .show(ui, |ui| {
                    ui.set_width(left_w - 34.0);
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    card_main(ui, p, info);
                });
        });
        ui.vertical(|ui| {
            ui.set_width(STUB_W);
            egui::Frame::none().inner_margin(Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                ui.set_width(STUB_W - 28.0);
                ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
                action = card_stub(ui, p, info, outcome, confirming, amount, tone, stage_text);
            });
        });
    });

    let rect = row.response.rect;
    ui.painter().set(bg, Shape::Vec(vec![
        Shape::rect_filled(rect, 10.0, p.surface),
        Shape::rect_stroke(rect, 10.0, Stroke::new(1.0, p.border)),
    ]));
    // 用卡片同样的圆角矩形裁出左缘 3px，竖条的圆角与卡片一致
    let strip = Rect::from_min_max(rect.min, pos2(rect.left() + 3.0, rect.bottom()));
    ui.painter()
        .with_clip_rect(strip.intersect(ui.clip_rect()))
        .rect_filled(rect, 10.0, p.tone(tone).0);
    let x = rect.right() - STUB_W;
    widgets::perforation(ui.painter(), pos2(x, rect.top()), pos2(x, rect.bottom()), p);
    action
}

fn card_main(ui: &mut Ui, p: &Palette, info: &RunningTaskInfo) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("No.{:04}", info.seq)).font(FontId::monospace(12.0)).color(p.text_3));
        let name_w = (ui.available_width() - 56.0).max(40.0);
        ui.allocate_ui_with_layout(vec2(name_w, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add(Label::new(RichText::new(&info.project_name).font(bold(15.0)).color(p.text)).truncate(true));
        });
        widgets::pill(ui, mode_label(info.grab_mode), Tone::Neutral);
    });

    let line2 = if info.target.is_empty() {
        info.account_name.clone()
    } else {
        format!("{} · {}", info.account_name, info.target)
    };
    ui.add(Label::new(RichText::new(line2).size(12.0).color(p.text_2)).truncate(true));

    let (line3, color) = match &info.last_event {
        Some(event) => (
            format!("{} · {}", event.text, ago(event.at)),
            if event.is_error { p.danger } else { p.text_2 },
        ),
        None => (format!("已运行 {}", mmss(info.elapsed_secs)), p.text_2),
    };
    ui.add(Label::new(RichText::new(line3).size(12.0).color(color)).truncate(true));
}

#[allow(clippy::too_many_arguments)]
fn card_stub(
    ui: &mut Ui,
    p: &Palette,
    info: &RunningTaskInfo,
    outcome: Option<&TaskOutcome>,
    confirming: bool,
    amount: Option<i64>,
    tone: Tone,
    stage_text: &str,
) -> Option<Action> {
    let mut action = None;
    let running = outcome.is_none();

    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        if confirming {
            // 存根只有 188 宽，放不下阶段标签和三个确认控件，确认期间整行换成确认
            widgets::small(ui, "确认停止？", p.text_2);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button_ex(ui, "取消", Kind::Ghost, Size::Sm, false, true).clicked() {
                    action = Some(Action::CancelStop);
                }
                if widgets::button_ex(ui, "停止", Kind::Danger, Size::Sm, false, true).clicked() {
                    action = Some(Action::ConfirmStop { task_id: info.task_id.clone(), seq: info.seq });
                }
            });
        } else {
            widgets::pill(ui, stage_text, tone);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if running && !is_cancelling(info)
                    && widgets::button_ex(ui, "停止", Kind::DangerGhost, Size::Sm, false, true).clicked()
                {
                    action = Some(Action::AskStop(info.task_id.clone()));
                }
                if widgets::button_ex(ui, "日志", Kind::Ghost, Size::Sm, false, true).clicked() {
                    action = Some(Action::OpenLog { seq: info.seq, account: info.account_name.clone() });
                }
            });
        }
    });

    let small_font = FontId::proportional(12.0);
    let metric = |ui: &mut Ui, before: &str, value: String, after: &str, color: Color32| {
        ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if !before.is_empty() {
                widgets::small(ui, before, p.text_2);
            }
            ui.label(RichText::new(value).font(FontId::monospace(24.0)).color(color));
            if !after.is_empty() {
                widgets::small(ui, after, p.text_2);
            }
        });
    };
    let busy = |ui: &mut Ui, text: &str| {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0).color(p.text_2));
            widgets::small(ui, text, p.text_2);
        });
    };

    match outcome {
        None => match info.phase {
            GrabPhase::Preparing => busy(ui, "正在准备"),
            GrabPhase::WaitingSale => {
                widgets::small(ui, "距开售", p.text_3);
                match info.sale_at {
                    Some(sale_at) => {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                        let left = sale_at - now;
                        let color = if left <= 10 { p.primary } else { p.text };
                        metric(ui, "", widgets::countdown_text(left), "", color);
                    }
                    None => metric(ui, "", "--:--:--".into(), "", p.text_3),
                }
            }
            GrabPhase::Grabbing => metric(ui, "已尝试", info.attempts.to_string(), "次", p.text),
            GrabPhase::Captcha => busy(ui, "正在处理验证码"),
            GrabPhase::Monitoring => metric(ui, "已检查", info.rounds.to_string(), "轮", p.text),
        },
        Some(TaskOutcome::Succeeded) => match amount {
            Some(fen) => {
                let text = format!("去付款 {}", widgets::money(fen));
                if widgets::button_ex(ui, &text, Kind::Primary, Size::Xl, false, true).clicked() {
                    action = Some(Action::Pay(info.task_id.clone()));
                }
            }
            None => {
                ui.label(RichText::new("下单成功").color(p.success));
            }
        },
        Some(TaskOutcome::Stopped) => {
            widgets::small(ui, format!("运行 {}", mmss(info.elapsed_secs)), p.text_3);
        }
        Some(TaskOutcome::Ended(reason)) => {
            let mut job = LayoutJob::single_section(reason.clone(), TextFormat::simple(small_font, p.danger));
            job.wrap.max_rows = 2;
            ui.add(Label::new(job).wrap(true));
        }
    }
    action
}

/// Sm 高度的开关按钮。framed：未选中为 Secondary 外观，选中为 primary_soft 底 + primary 描边；
/// 否则为 Ghost 外观，选中时 primary 字
fn toggle_button(ui: &mut Ui, text: &str, on: bool, framed: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(13.0);
    let text_w = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.text).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + 20.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let active = resp.hovered() || resp.is_pointer_button_down_on();
        let (fill, stroke, fg) = match (on, framed) {
            (true, true) => (p.primary_soft, Some(p.primary), p.primary),
            (false, true) => (
                if active { p.sunken } else { p.surface },
                Some(if active { p.text_3 } else { p.border_strong }),
                p.text,
            ),
            (true, false) => (if active { p.sunken } else { Color32::TRANSPARENT }, None, p.primary),
            (false, false) => (
                if active { p.sunken } else { Color32::TRANSPARENT },
                None,
                if active { p.text } else { p.text_2 },
            ),
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 6.0, fill);
        if let Some(color) = stroke {
            painter.rect_stroke(rect, 6.0, Stroke::new(1.0, color));
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), 8.0, Stroke::new(2.0, p.primary));
        }
        painter.text(rect.center(), Align2::CENTER_CENTER, text, font, fg);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ---------- 日志面板 ----------

fn log_panel(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    egui::TopBottomPanel::bottom("log_panel")
        .resizable(true)
        .default_height(220.0)
        .height_range(120.0..=480.0)
        .frame(egui::Frame::none().fill(p.sunken).inner_margin(Margin { left: 24.0, right: 24.0, top: 0.0, bottom: 8.0 }))
        .show_inside(ui, |ui| {
            let log_action = log_toolbar(app, ui);
            if let Some(log_action) = log_action {
                apply_log_action(app, ui.ctx(), log_action);
            }
            app.logs.refilter(&app.log_filter);
            log_rows(app, ui, p);
        });
}

fn log_toolbar(app: &mut Myapp, ui: &mut Ui) -> Option<LogAction> {
    let mut action = None;
    // 窄窗口下一行放不下，改为自动换行（右侧按钮跟在搜索框后）
    let wide = ui.available_width() >= 860.0;
    let left = |ui: &mut Ui, app: &mut Myapp| {
        widgets::segmented(
            ui,
            &mut app.log_filter.level,
            &[
                (LevelFilter::All, "全部"),
                (LevelFilter::Info, "信息"),
                (LevelFilter::Warn, "警告"),
                (LevelFilter::Error, "错误"),
            ],
        );
        let selected = app.log_filter.label.as_deref().map(display_label).unwrap_or_else(|| "全部任务".to_owned());
        let labels = &app.logs.labels;
        let filter_label = &mut app.log_filter.label;
        egui::ComboBox::from_id_source("log_label_filter")
            .width(150.0)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut *filter_label, None, "全部任务");
                for label in labels {
                    ui.selectable_value(&mut *filter_label, Some(label.clone()), display_label(label));
                }
            });
        widgets::text_field(
            ui,
            &mut app.log_filter.search,
            FieldOpts { hint: "搜索日志", width: Some(180.0), id: Some("log_search"), ..Default::default() },
        );
    };
    let follow = app.log_filter.follow;
    let mut right = |ui: &mut Ui, rtl: bool| {
        let mut items: Vec<(&str, LogAction)> = vec![
            ("跟随最新", LogAction::ToggleFollow),
            ("复制", LogAction::Copy),
            ("打开日志文件夹", LogAction::OpenFolder),
            ("清空", LogAction::Clear),
        ];
        if rtl {
            items.reverse();
        }
        for (text, item) in items {
            let on = matches!(item, LogAction::ToggleFollow) && follow;
            if toggle_button(ui, text, on, false).clicked() {
                action = Some(item);
            }
        }
    };
    if wide {
        ui.horizontal(|ui| {
            ui.set_min_height(40.0);
            left(ui, app);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                right(ui, true);
            });
        });
    } else {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            left(ui, app);
            right(ui, false);
        });
        ui.add_space(2.0);
    }
    action
}

fn apply_log_action(app: &mut Myapp, ctx: &egui::Context, action: LogAction) {
    match action {
        LogAction::ToggleFollow => app.log_filter.follow = !app.log_filter.follow,
        LogAction::Copy => {
            app.logs.refilter(&app.log_filter);
            let text = app
                .logs
                .filtered
                .iter()
                .map(|&i| app.logs.lines[i].raw.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            widgets::copy_text(ctx, text);
            app.toasts.success(format!("已复制 {} 行日志", app.logs.filtered.len()));
        }
        LogAction::OpenFolder => {
            if let Err(e) = open_log_folder() {
                log::error!("打开日志文件夹失败: {}", e);
                app.toasts.danger(format!("打开日志文件夹失败：{}", e));
            }
        }
        LogAction::Clear => app.logs.clear(),
    }
}

/// 日志目录由 common/record_log.rs 写在工作目录下的 Log
fn open_log_folder() -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    Command::new(program).arg("Log").spawn().map(|_| ())
}

fn log_rows(app: &Myapp, ui: &mut Ui, p: &Palette) {
    if app.logs.lines.is_empty() || app.logs.filtered.is_empty() {
        ui.add_space(12.0);
        let text = if app.logs.lines.is_empty() { "暂无日志" } else { "没有符合条件的日志" };
        widgets::small(ui, text, p.text_3);
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_source("log_rows")
        .auto_shrink([false, false])
        .stick_to_bottom(app.log_filter.follow)
        .show_rows(ui, LOG_ROW_H, app.logs.filtered.len(), |ui, range| {
            for i in range {
                log_row(ui, p, &app.logs.lines[app.logs.filtered[i]]);
            }
        });
}

/// 单行不换行：时间 · 级别块 · 任务标签 · 内容，整行手绘以保证行高一致
fn log_row(ui: &mut Ui, p: &Palette, line: &LogLine) {
    let font = FontId::monospace(12.5);
    let tag_font = FontId::monospace(10.5);
    let gap = 8.0;
    let time_w = 62.0;
    let tag_w = 44.0;
    let painter = ui.painter();
    let msg = painter.layout_no_wrap(line.msg.clone(), font.clone(), p.text);
    let label = line
        .label
        .as_deref()
        .map(|l| painter.layout_no_wrap(display_label(l), font.clone(), p.info));
    let label_w = label.as_ref().map_or(0.0, |g| g.size().x + gap);
    let total = time_w + gap + tag_w + gap + label_w + msg.size().x;
    let (rect, _) = ui.allocate_exact_size(vec2(total.max(ui.available_width()), LOG_ROW_H), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let y = rect.center().y;
    let mut x = rect.left();
    painter.text(pos2(x, y), Align2::LEFT_CENTER, &line.time, font, p.text_3);
    x += time_w + gap;

    let (tone, tag) = match line.level {
        Level::Debug => (Tone::Neutral, "DEBUG"),
        Level::Info => (Tone::Neutral, "INFO"),
        Level::Warn => (Tone::Warning, "WARN"),
        Level::Error => (Tone::Danger, "ERROR"),
    };
    let (fg, bg) = p.tone(tone);
    let tag_rect = Rect::from_min_size(pos2(x, y - 8.0), vec2(tag_w, 16.0));
    painter.rect_filled(tag_rect, 4.0, bg);
    painter.text(tag_rect.center(), Align2::CENTER_CENTER, tag, tag_font, fg);
    x += tag_w + gap;

    if let Some(galley) = label {
        let w = galley.size().x;
        painter.galley(pos2(x, y - galley.size().y / 2.0), galley);
        x += w + gap;
    }
    let h = msg.size().y;
    painter.galley(pos2(x, y - h / 2.0), msg);
}
