mod config;
mod storage;
mod task;

use storage::TaskStore;
use task::Priority;

fn main() {
    let mut store = TaskStore::new();

    let id1 = store.add_task("Set up CI pipeline", Priority::High);
    let id2 = store.add_task("Write documentation", Priority::Low);
    let id3 = store.add_task("Fix login bug", Priority::Critical);

    store.complete_task(&id1);

    println!("=== Task Tracker ===");
    println!("Total tasks: {}", store.count());
    println!("Pending: {}", store.pending_count());
    println!("Completed: {}", store.completed_count());

    println!("\nPending tasks (sorted by priority):");
    for task in store.pending_by_priority() {
        println!("  [{:?}] {} - {}", task.priority, task.id, task.title);
    }
}
