use std::collections::HashMap;

use crate::task::{Priority, Status, Task};

pub struct TaskStore {
    tasks: HashMap<String, Task>,
    next_id: u32,
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn add_task(&mut self, title: &str, priority: Priority) -> String {
        let id = format!("TASK-{:04}", self.next_id);
        self.next_id += 1;
        let task = Task::new(id.clone(), title, priority);
        self.tasks.insert(id.clone(), task);
        id
    }

    pub fn complete_task(&mut self, id: &str) -> bool {
        if let Some(task) = self.tasks.get_mut(id) {
            task.complete();
            true
        } else {
            false
        }
    }

    pub fn get_task(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn count(&self) -> usize {
        self.tasks.len()
    }

    pub fn pending_count(&self) -> usize {
        self.tasks.values().filter(|t| t.is_pending()).count()
    }

    pub fn completed_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|t| t.status == Status::Completed)
            .count()
    }

    pub fn pending_by_priority(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.values().filter(|t| t.is_pending()).collect();
        tasks.sort_by(|a, b| b.priority.cmp(&a.priority));
        tasks
    }
}
