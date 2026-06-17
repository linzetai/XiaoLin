use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub priority: Priority,
    pub status: Status,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Task {
    pub fn new(id: String, title: impl Into<String>, priority: Priority) -> Self {
        Self {
            id,
            title: title.into(),
            priority,
            status: Status::Pending,
            created_at: Utc::now(),
            completed_at: None,
        }
    }

    pub fn complete(&mut self) {
        self.status = Status::Completed;
        self.completed_at = Some(Utc::now());
    }

    pub fn is_pending(&self) -> bool {
        self.status == Status::Pending
    }
}
