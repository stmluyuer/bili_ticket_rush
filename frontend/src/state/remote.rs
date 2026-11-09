use std::time::Instant;

#[derive(Clone, Debug)]
pub enum Remote<T> {
    Idle,
    Loading { task_id: String, since: Instant },
    Ready { value: T, at: Instant },
    Failed { message: String, at: Instant },
}

impl<T> Default for Remote<T> {
    fn default() -> Self { Self::Idle }
}

impl<T> Remote<T> {
    pub fn cooldown_left(&self, secs: u64) -> u64 {
        let at = match self {
            Self::Idle => return 0,
            Self::Loading { since, .. } => since,
            Self::Ready { at, .. } | Self::Failed { at, .. } => at,
        };
        secs.saturating_sub(at.elapsed().as_secs())
    }

    pub fn value(&self) -> Option<&T> {
        match self { Self::Ready { value, .. } => Some(value), _ => None }
    }
}
