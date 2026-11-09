use std::cmp::Reverse;
use std::collections::{HashSet, VecDeque};
use std::time::{Instant, SystemTime};
use common::taskmanager::{FinishedTaskInfo, OrderState, RunningTaskInfo, TaskManager};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TasksTab { Running, Finished }

#[derive(Clone, Debug)]
pub struct PaymentInfo {
    pub task_id: String,
    pub uid: i64,
    pub order_id: Option<String>,
    pub code_url: String,
    pub project_name: String,
    pub screen_name: String,
    pub ticket_name: String,
    pub count: i32,
    pub amount_fen: i64,
    pub at: SystemTime,
}

#[derive(Default)]
pub struct TaskBoard {
    pub running: Vec<RunningTaskInfo>,
    pub finished: Vec<FinishedTaskInfo>,
    finished_ids: HashSet<String>,
    pub payments: VecDeque<PaymentInfo>,
    pub payment_index: usize,
    payment_dialog_pending: bool,
    pub submitted: Vec<(String, i64, String, String)>,
    pub stop_confirm: Option<(String, Instant)>,
}

impl TaskBoard {
    /// 返回本次是否新增了付款提醒。
    pub fn refresh(&mut self, manager: &mut dyn TaskManager) -> bool {
        self.running = manager.list_running_tasks();
        self.running.sort_by_key(|info| info.seq);
        let previous_payments = self.payments.len();
        for finished in manager.take_finished_tasks() {
            let info = &finished.info;
            if !self.finished_ids.insert(info.task_id.clone()) {
                continue;
            }
            if let Some(order) = &info.order {
                if let OrderState::AwaitingPayment(pay) = &order.state {
                    let confirm = &order.confirm;
                    self.payments.push_back(PaymentInfo {
                        task_id: info.task_id.clone(),
                        uid: info.uid,
                        order_id: order.order_id.clone(),
                        code_url: pay.code_url.clone(),
                        project_name: confirm.project_name.clone(),
                        screen_name: confirm.screen_name.clone(),
                        ticket_name: confirm.ticket_info.name.clone(),
                        count: confirm.count,
                        amount_fen: confirm.pay_money,
                        at: finished.finished_at,
                    });
                }
            }
            self.finished.push(finished);
        }
        // 任务可能在读取进行中快照后结束。同一帧以已经取到的终态为准。
        self.running.retain(|info| !self.finished_ids.contains(&info.task_id));
        if self.stop_confirm.as_ref().is_some_and(|(id, at)| {
            self.finished_ids.contains(id) || at.elapsed().as_secs() >= 3
        }) {
            self.stop_confirm = None;
        }
        self.finished.sort_by_key(|finished| {
            let payable = matches!(
                finished.info.order.as_ref().map(|order| &order.state),
                Some(OrderState::AwaitingPayment(_))
            );
            Reverse((payable, finished.finished_at))
        });
        let new_payment = self.payments.len() > previous_payments;
        self.payment_dialog_pending |= new_payment;
        new_payment
    }

    pub fn take_pending_payment_dialog(&mut self) -> bool {
        std::mem::take(&mut self.payment_dialog_pending)
    }

    pub fn clear_finished(&mut self) {
        // 关闭提醒后，历史仍保留订单状态与核对信息。
        self.finished.retain(|finished| finished.info.order.is_some());
    }

    pub fn dismiss_payment_reminder(&mut self) {
        self.payments.remove(self.payment_index);
        self.payment_index = self.payment_index.min(self.payments.len().saturating_sub(1));
    }

    /// 任务编号（进行中或已结束），找不到时为 None
    pub fn seq_of(&self, task_id: &str) -> Option<u64> {
        self.running
            .iter()
            .chain(self.finished.iter().map(|f| &f.info))
            .find(|t| t.task_id == task_id)
            .map(|t| t.seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::taskmanager::{GrabPhase, OrderContext, TaskOutcome, TaskRequest, TaskResult, TaskStatus};
    use common::ticket::{CheckFakeResultData, ConfirmTicketInfo, ConfirmTicketResult};
    use crate::app::Dialog;
    use crate::ui::dialogs::LoginState;

    #[derive(Default)]
    struct TaskSnapshots {
        running: Vec<RunningTaskInfo>,
        finished: Vec<FinishedTaskInfo>,
    }

    impl TaskManager for TaskSnapshots {
        fn new() -> Self { Self::default() }
        fn submit_task(&mut self, _: TaskRequest) -> Result<String, String> { unreachable!() }
        fn get_results(&mut self) -> Vec<TaskResult> { unreachable!() }
        fn cancel_task(&mut self, _: &str) -> Result<(), String> { unreachable!() }
        fn get_task_status(&self, _: &str) -> Option<TaskStatus> { unreachable!() }
        fn list_running_tasks(&self) -> Vec<RunningTaskInfo> { self.running.clone() }
        fn take_finished_tasks(&mut self) -> Vec<FinishedTaskInfo> { std::mem::take(&mut self.finished) }
        fn shutdown(&mut self) { unreachable!() }
    }

    fn finished(task_id: &str, uid: i64, outcome: TaskOutcome, order: Option<OrderContext>) -> FinishedTaskInfo {
        FinishedTaskInfo {
            info: RunningTaskInfo {
                task_id: task_id.into(),
                seq: 1,
                uid,
                account_name: "同名账号".into(),
                project_name: "项目".into(),
                grab_mode: 1,
                status: TaskStatus::Completed(true),
                elapsed_secs: 1,
                phase: GrabPhase::CheckingOrder,
                sale_at: None,
                sale_at_ms: None,
                remaining_ms: None,
                attempts: 1,
                rounds: 0,
                last_event: None,
                target: String::new(),
                project_id: "1".into(),
                ticket_id: "3".into(),
                order,
            },
            outcome,
            finished_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn order(order_id: Option<&str>, pay_money: i64, state: OrderState) -> OrderContext {
        OrderContext {
            order_id: order_id.map(str::to_owned),
            query_token: Some("query-token".into()),
            project_id: "1".into(),
            screen_id: "2".into(),
            ticket_id: "3".into(),
            confirm: ConfirmTicketResult {
                count: 2,
                pay_money,
                project_name: "项目".into(),
                screen_name: "场次".into(),
                ticket_info: ConfirmTicketInfo { name: "票档".into(), count: 2, price: 2000 },
            },
            state,
        }
    }

    fn payable() -> OrderState {
        OrderState::AwaitingPayment(CheckFakeResultData { sign: String::new(), code_url: "test-pay-link".into() })
    }

    #[test]
    fn refresh_uses_confirmed_amount_and_original_uid_once() {
        let stopped = finished("task-1", 123, TaskOutcome::Stopped, Some(order(None, 0, payable())));
        let succeeded = finished("task-2", 456, TaskOutcome::Succeeded, Some(order(Some("42"), 105, payable())));
        let mut manager = TaskSnapshots { finished: vec![stopped.clone(), stopped, succeeded], ..Default::default() };
        let mut board = TaskBoard::default();

        assert!(board.refresh(&mut manager));
        assert_eq!(board.finished.len(), 2);
        assert_eq!(board.payments.len(), 2);
        let first = &board.payments[0];
        assert_eq!((first.task_id.as_str(), first.uid, first.order_id.as_deref(), first.amount_fen), ("task-1", 123, None, 0));
        assert_eq!(first.code_url, "test-pay-link");
        let second = &board.payments[1];
        assert_eq!((second.task_id.as_str(), second.uid, second.order_id.as_deref(), second.amount_fen), ("task-2", 456, Some("42"), 105));
        assert_eq!(crate::ui::widgets::money(first.amount_fen), "¥0.00");
        assert_eq!(crate::ui::widgets::money(second.amount_fen), "¥1.05");
        assert!(!board.refresh(&mut manager));
        assert_eq!(board.payments.len(), 2);
    }

    #[test]
    fn dismissing_and_clearing_preserve_pending_and_payable_orders() {
        let pending = finished("pending", 123, TaskOutcome::Stopped, Some(order(Some("42"), 105, OrderState::PendingConfirmation)));
        let payment = finished("payment", 456, TaskOutcome::Succeeded, Some(order(None, 105, payable())));
        let failure = finished("failure", 456, TaskOutcome::Ended("明确拒绝".into()), None);
        let mut manager = TaskSnapshots {
            finished: vec![pending, payment.clone(), failure.clone()],
            ..Default::default()
        };
        let mut board = TaskBoard::default();

        assert!(board.refresh(&mut manager));
        assert_eq!(board.payments.len(), 1);
        board.dismiss_payment_reminder();
        board.clear_finished();
        manager.finished.extend([payment, failure]);
        assert!(!board.refresh(&mut manager));
        assert!(board.payments.is_empty());
        assert_eq!(board.finished.len(), 2);
        let pending = board.finished.iter().find(|record| record.info.task_id == "pending").unwrap();
        assert!(matches!(pending.outcome, TaskOutcome::Stopped));
        assert_eq!(pending.info.order.as_ref().unwrap().order_id.as_deref(), Some("42"));
        let payment = board.finished.iter().find(|record| record.info.task_id == "payment").unwrap();
        assert!(matches!(payment.info.order.as_ref().unwrap().state, OrderState::AwaitingPayment(_)));
    }

    #[test]
    fn refresh_prefers_finished_over_older_running_snapshot() {
        let completed = finished("completed", 123, TaskOutcome::Stopped, None);
        let mut manager = TaskSnapshots {
            running: vec![completed.info.clone(), finished("running", 123, TaskOutcome::Stopped, None).info],
            finished: vec![completed],
        };
        let mut board = TaskBoard {
            stop_confirm: Some(("completed".into(), Instant::now())),
            ..Default::default()
        };

        assert!(!board.refresh(&mut manager));
        assert_eq!(board.running.len(), 1);
        assert_eq!(board.running[0].task_id, "running");
        assert_eq!(board.finished[0].info.task_id, "completed");
        assert!(board.stop_confirm.is_none());
    }

    #[test]
    fn payment_dialog_waits_for_login_and_keeps_current_order_selected() {
        let first = finished("first", 123, TaskOutcome::Succeeded, Some(order(Some("42"), 105, payable())));
        let mut manager = TaskSnapshots { finished: vec![first], ..Default::default() };
        let mut board = TaskBoard::default();
        let mut dialog = Dialog::Login(LoginState { qr_task: Some("login".into()), ..Default::default() });

        assert!(board.refresh(&mut manager));
        dialog.show_pending_payment(&mut board);
        assert!(matches!(&dialog, Dialog::Login(state) if state.qr_task.as_deref() == Some("login")));

        dialog = Dialog::None;
        dialog.show_pending_payment(&mut board);
        assert!(matches!(dialog, Dialog::Payment));
        assert_eq!(board.payments[board.payment_index].task_id, "first");

        let second = finished("second", 456, TaskOutcome::Succeeded, Some(order(Some("43"), 260, payable())));
        manager.finished.push(second);
        assert!(board.refresh(&mut manager));
        dialog.show_pending_payment(&mut board);
        assert_eq!(board.payments.len(), 2);
        assert_eq!(board.payments[board.payment_index].task_id, "first");

        dialog = Dialog::None;
        dialog.show_pending_payment(&mut board);
        assert!(matches!(dialog, Dialog::None));
        assert_eq!(board.payments.len(), 2);
    }
}
