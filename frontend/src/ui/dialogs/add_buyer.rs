use eframe::egui::{self, Ui};

use crate::app::{Dialog, Myapp};
use crate::state::jobs::BuyerForm;
use crate::ui::dialogs::{AddBuyerState, FieldErrors};
use crate::ui::modal::{self, ModalAction};
use crate::ui::theme::{palette, Tone};
use crate::ui::widgets::{self, button_ex, FieldOpts, Filter, Kind, Size};

pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    let mut st = match std::mem::replace(&mut app.dialog, Dialog::None) {
        Dialog::AddBuyer(st) => st,
        other => {
            app.dialog = other;
            return;
        }
    };
    let mut save = false;
    let mut cancel = false;
    let action = modal::modal(ctx, "add_buyer_dialog", "添加购票人", 480.0, true, |ui| {
        form(ui, &mut st, &mut save, &mut cancel);
        ModalAction::None
    });
    if save && !st.busy {
        submit(app, &mut st);
    }
    if matches!(app.dialog, Dialog::None) {
        app.dialog = Dialog::AddBuyer(st);
        if action == ModalAction::Close || cancel {
            app.close_dialog();
        }
    }
}

fn form(ui: &mut Ui, st: &mut AddBuyerState, save: &mut bool, cancel: &mut bool) {
    let p = palette(ui.ctx());
    if let Some(error) = &st.server_error {
        widgets::callout(ui, error, Tone::Danger, None);
    }

    let name = widgets::text_field(ui, &mut st.name, FieldOpts {
        label: Some("姓名"),
        hint: "与证件一致的姓名",
        error: st.errors.name.clone(),
        id: Some("buyer_name"),
        ..Default::default()
    });
    if name.changed() {
        st.errors.name = None;
    }

    let phone = widgets::text_field(ui, &mut st.phone, FieldOpts {
        label: Some("手机号"),
        hint: "11 位手机号",
        error: st.errors.phone.clone(),
        filter: Filter::Digits,
        id: Some("buyer_phone"),
        ..Default::default()
    });
    st.phone.truncate(11);
    if phone.changed() {
        st.errors.phone = None;
    }

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        widgets::small(ui, "证件类型", p.text_2);
        let types = [(0usize, "身份证"), (1, "护照"), (2, "港澳通行证"), (3, "台湾通行证")];
        if widgets::segmented(ui, &mut st.id_type, &types) {
            st.errors.id_number = None;
        }
    });

    let hint = match st.id_type {
        0 => "18 位身份证号码",
        1 => "护照号码",
        2 => "港澳通行证号码",
        _ => "台湾通行证号码",
    };
    let id_number = widgets::text_field(ui, &mut st.id_number, FieldOpts {
        label: Some("证件号码"),
        hint,
        error: st.errors.id_number.clone(),
        filter: Filter::Token,
        id: Some("buyer_id_number"),
        ..Default::default()
    });
    if id_number.changed() {
        st.errors.id_number = None;
    }

    ui.checkbox(&mut st.as_default, "设为默认购票人");

    modal::footer(ui, |ui| {
        let text = if st.busy { "保存中…" } else { "保存" };
        *save = button_ex(ui, text, Kind::Primary, Size::Md, st.busy, true).clicked();
        *cancel = button_ex(ui, "取消", Kind::Secondary, Size::Md, false, true).clicked();
    });
}

fn validate(st: &AddBuyerState) -> FieldErrors {
    let mut errors = FieldErrors::default();
    if st.name.trim().is_empty() {
        errors.name = Some("请填写姓名".into());
    }
    if st.phone.len() != 11 {
        errors.phone = Some("请输入 11 位手机号".into());
    }
    if st.id_type == 0 {
        if !is_id_card(&st.id_number) {
            errors.id_number = Some("身份证号应为 18 位".into());
        }
    } else if st.id_number.is_empty() {
        errors.id_number = Some("请填写证件号码".into());
    }
    errors
}

/// 17 位数字 + 数字或 X
fn is_id_card(id: &str) -> bool {
    let chars: Vec<char> = id.chars().collect();
    chars.len() == 18
        && chars[..17].iter().all(|c| c.is_ascii_digit())
        && (chars[17].is_ascii_digit() || chars[17] == 'X' || chars[17] == 'x')
}

fn submit(app: &mut Myapp, st: &mut AddBuyerState) {
    st.errors = validate(st);
    if st.errors.name.is_some() || st.errors.phone.is_some() || st.errors.id_number.is_some() {
        return;
    }
    let Some(cookie_manager) = app.account(st.uid).and_then(|a| a.cookie_manager.clone()) else {
        st.server_error = Some("账号登录信息失效，请重新登录".into());
        return;
    };
    let personal_id = if st.id_type == 0 { st.id_number.to_ascii_uppercase() } else { st.id_number.clone() };
    st.job_id = Some(app.jobs.spawn_add_buyer(st.uid, cookie_manager, BuyerForm {
        name: st.name.trim().to_owned(),
        tel: st.phone.clone(),
        id_type: st.id_type,
        personal_id,
        is_default: st.as_default,
    }));
    st.server_error = None;
    st.busy = true;
}
