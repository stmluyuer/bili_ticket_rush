use eframe::egui::{self, vec2, Align2, FontId, Margin, RichText, Sense, Ui};

use common::taskmanager::TaskManager_debug;

use crate::app::Myapp;
use crate::ui::theme::{bold, palette, Palette, Tone};
use crate::ui::widgets::{button_ex, card, copy_text, open_url, page_title, section_title, small, Kind, Size};

const REPO_URL: &str = "https://github.com/biliticket/bili_ticket_rush";
const VC_REDIST_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

const STEPS: [&str; 4] = [
    "在“账号”页登录 B 站账号并添加购票人",
    "在“抢票”页粘贴项目链接，选择抢票方式、场次票种和购票人",
    "点“开始”，在“任务”页查看进度",
    "抢到后在弹出的付款框扫码付款",
];

pub fn render(app: &mut Myapp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    egui::ScrollArea::vertical()
        .id_source("about_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(Margin { left: 24.0, right: 24.0, top: 20.0, bottom: 24.0 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(720.0));
                    page_title(ui, "关于", |_| {});
                    ui.add_space(8.0);
                    version(app, ui, &p);
                    ui.add_space(8.0);
                    card(ui, |ui| {
                        section_title(ui, "声明", |_| {});
                        wrapped(ui, TaskManager_debug(), &p);
                    });
                    ui.add_space(8.0);
                    announcements(app, ui, &p);
                    ui.add_space(8.0);
                    steps(ui, &p);
                    ui.add_space(8.0);
                    faq(ui, &p);
                });
        });
}

fn version(app: &mut Myapp, ui: &mut Ui, p: &Palette) {
    let mut copied = false;
    card(ui, |ui| {
        ui.label(RichText::new(format!("BiliTicketRush {}", app.version)).font(bold(18.0)).color(p.text));
        ui.horizontal(|ui| {
            ui.label(RichText::new(REPO_URL).font(FontId::monospace(13.0)).color(p.text_2));
            if button_ex(ui, "复制", Kind::Ghost, Size::Sm, false, true).clicked() {
                copy_text(ui.ctx(), REPO_URL);
                copied = true;
            }
            if button_ex(ui, "打开", Kind::Ghost, Size::Sm, false, true).clicked() {
                open_url(ui.ctx(), REPO_URL);
            }
        });
    });
    if copied {
        app.toasts.push(Tone::Success, "已复制项目地址", None, None);
    }
}

fn announcements(app: &Myapp, ui: &mut Ui, p: &Palette) {
    let list: Vec<&String> = [&app.announce1, &app.announce2].into_iter().flatten().collect();
    card(ui, |ui| {
        section_title(ui, "公告", |_| {});
        if list.is_empty() {
            small(ui, "暂无公告", p.text_3);
            return;
        }
        for (i, text) in list.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            wrapped(ui, text.as_str(), p);
        }
    });
}

fn steps(ui: &mut Ui, p: &Palette) {
    card(ui, |ui| {
        section_title(ui, "使用步骤", |_| {});
        for (i, text) in STEPS.iter().enumerate() {
            ui.horizontal_top(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 11.0, p.primary_soft);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, (i + 1).to_string(), bold(12.0), p.primary);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(*text).color(p.text)).wrap(true));
                });
            });
        }
    });
}

fn faq(ui: &mut Ui, p: &Palette) {
    card(ui, |ui| {
        section_title(ui, "常见问题", |_| {});
        question(ui, p, "提示缺少 MSVCP140.dll");
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("安装 Visual C++ 运行库后重启电脑：").color(p.text_2));
            ui.hyperlink_to("VC_redist.x64.exe", VC_REDIST_URL);
        });
        ui.add_space(4.0);
        question(ui, p, "电脑没有显卡或启动闪退");
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("安装 Mesa3D 软件渲染器，步骤见项目 README 的“常见问题”：").color(p.text_2));
            ui.hyperlink_to("打开项目主页", REPO_URL);
        });
        ui.add_space(4.0);
        question(ui, p, "看不到日志");
        wrapped(ui, "在“任务”页点右上角的“日志”，打开日志面板。", p);
    });
}

fn question(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).font(bold(14.0)).color(p.text));
}

fn wrapped(ui: &mut Ui, text: impl Into<String>, p: &Palette) {
    ui.add(egui::Label::new(RichText::new(text.into()).color(p.text_2)).wrap(true));
}
