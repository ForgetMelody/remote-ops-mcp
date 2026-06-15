use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Job 生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Starting,
    Running,
    Exited,
    TimedOut,
    Cancelled,
    Failed,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Exited | Self::TimedOut | Self::Cancelled | Self::Failed
        )
    }
}
