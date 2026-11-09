use std::collections::VecDeque;
use std::time::{Instant, SystemTime};
use common::taskmanager::{FinishedTaskInfo, RunningTaskInfo, TaskManager, TaskOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TasksTab { Running, Finished }

#[derive(Clone, Debug)]
pub struct PaymentInfo {
    pub task_id: String,
    pub uid: i64,
    pub order_id: String,
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
    pub payments: VecDeque<PaymentInfo>,
    pub payment_index: usize,
    pub submitted: Vec<(String, i64, String, String)>,
    pub stop_confirm: Option<(String, Instant)>,
}

impl TaskBoard {
    pub fn refresh(&mut self, manager: &mut dyn TaskManager) {
        self.running = manager.list_running_tasks();
        self.running.sort_by_key(|info| info.seq);
        self.finished.extend(manager.take_finished_tasks());
        self.finished.sort_by(|a, b| {
            let a_success = matches!(a.outcome, TaskOutcome::Succeeded);
            let b_success = matches!(b.outcome, TaskOutcome::Succeeded);
            b_success.cmp(&a_success).then_with(|| b.finished_at.cmp(&a.finished_at))
        });
        if self.stop_confirm.as_ref().map_or(false, |(_, at)| at.elapsed().as_secs() >= 3) {
            self.stop_confirm = None;
        }
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
