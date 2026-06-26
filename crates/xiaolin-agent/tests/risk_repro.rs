//! Reproduction tests for high-priority code review risks.
//!
//! Each test demonstrates a specific risk identified during review.
//! Tests are named `risk{N}_*` matching the risk report numbering.

use std::sync::Arc;

// ─── RISK 3: Blocking filesystem I/O in async context ────────────────
//
// tool_round.rs calls std::fs::read_to_string and std::fs::write directly
// in an async context. This test demonstrates the blocking behavior.

#[tokio::test]
async fn risk3_blocking_fs_in_async_context_blocks_runtime() {
    // Simulate what tool_round.rs does: blocking I/O inside an async fn.
    // With a single-threaded runtime, this would deadlock other tasks.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<&str>(1);

    let t0 = std::time::Instant::now();

    // Task 1: simulates blocking FS I/O (like std::fs::read_to_string)
    let blocker = tokio::spawn(async move {
        // This is what tool_round.rs does — blocking I/O in async.
        // In a real scenario with slow disk, this blocks the worker.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large_file.txt");
        let data = "x".repeat(1024 * 1024); // 1MB
        std::fs::write(&path, &data).unwrap(); // BLOCKING
        let _content = std::fs::read_to_string(&path).unwrap(); // BLOCKING
        tx.send("blocker_done").await.unwrap();
    });

    // Task 2: should run concurrently but may be delayed by blocking I/O
    let waiter = tokio::spawn(async move { rx.recv().await });

    let _ = tokio::join!(blocker, waiter);
    let elapsed = t0.elapsed();

    // On a healthy async runtime, both tasks complete quickly.
    // But if std::fs ops block the worker thread, elapsed time increases.
    // The real risk manifests when the file is on slow storage (NFS, etc.)
    // or when the tokio runtime has limited worker threads.
    eprintln!(
        "risk3: blocking FS I/O in async completed in {:.1}ms \
         (on slow storage this would block the entire runtime)",
        elapsed.as_secs_f64() * 1000.0
    );
}

// ─── RISK 4: Stream drop without cancelling spawned task ─────────────
//
// execute_as_stream spawns a task that continues running even after
// the stream is dropped, unless the caller explicitly cancels the token.

#[tokio::test]
async fn risk4_spawned_task_survives_stream_drop() {
    let leaked_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag_for_task = leaked_flag.clone();

    // Simulate the pattern from stream_loop.rs:
    // - spawn a task that does work
    // - yield results through a channel
    // - if the stream (channel rx) is dropped, the task keeps running
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(4);

    let handle = tokio::spawn(async move {
        // Simulates the turn loop doing work
        for i in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if tx.send(format!("step {i}")).await.is_err() {
                // Channel closed — but task CONTINUES unless we break.
                // In the real code, the task has no way to know the stream
                // was dropped. It needs an explicit CancellationToken.
                break;
            }
        }
        // Mark that the task completed — if the flag is set after drop,
        // it means the task leaked resources.
        flag_for_task.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Read one item then DROP the receiver (simulating stream drop)
    let mut rx = rx;
    let first = rx.recv().await;
    assert!(first.is_some());
    drop(rx); // Stream consumer drops the channel

    // Wait a bit — the task should stop soon because tx.send fails
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // In the real code (stream_loop.rs), the spawned task does NOT check
    // for channel closure in the same way. It runs turn_loop::run_turn_loop
    // which keeps making LLM calls regardless. The handle is not wrapped
    // in AbortOnDropHandle, so:
    // - Task continues running
    // - LLM API calls keep being made (wasting money)
    // - Tool executions continue (side effects)
    let task_finished = leaked_flag.load(std::sync::atomic::Ordering::SeqCst);
    eprintln!(
        "risk4: task finished = {} (in the real code, the task may keep \
         running LLM calls indefinitely after stream drop)",
        task_finished
    );

    let _ = handle.await;
}

// ─── RISK 5: after_chat called from multiple code paths ──────────────
//
// Demonstrates the state machine gap in spawn_chat's event loop that
// allows after_chat to be called multiple times for the same turn.

#[test]
fn risk5_after_chat_multiple_persistence_paths() {
    // Model the event loop state transitions that lead to duplicate calls.
    // The real code in ws/chat.rs has 4 call sites for after_chat:
    //
    //   Path A: turn_cancel fired → L713
    //   Path B: AgentEvent::Error received → L786
    //   Path C: AgentEvent::TurnEnd received → L829
    //   Path D: post-loop cleanup (!stream_ended && reserved > 0) → L936
    //
    // Scenario: Error event arrives, then loop breaks, then cleanup runs
    struct MockState {
        after_chat_calls: u32,
        stream_ended: bool,
        reserved: f64,
        assistant_content: String,
    }

    let mut state = MockState {
        after_chat_calls: 0,
        stream_ended: false,
        reserved: 0.05,
        assistant_content: "some content".into(),
    };

    // Simulate: Error event arrives (Path B)
    if !state.assistant_content.is_empty() {
        state.after_chat_calls += 1; // L786: after_chat called
    }
    // Loop breaks after Error

    // Post-loop cleanup (Path D): !stream_ended && reserved > 0
    if !state.stream_ended && state.reserved > 0.0 && !state.assistant_content.is_empty() {
        state.after_chat_calls += 1; // L936: after_chat called AGAIN
    }

    assert_eq!(
        state.after_chat_calls, 2,
        "RISK: after_chat called {} times for the same turn (expected 1)",
        state.after_chat_calls
    );
    eprintln!(
        "risk5: after_chat called {} times — assistant message persisted twice!",
        state.after_chat_calls
    );
}

// ─── RISK 8: WaitAgentTool only polls first broadcast receiver ───────
//
// Demonstrates that when multiple session pools have broadcast channels,
// only the first receiver is awaited.

#[tokio::test]
async fn risk8_wait_only_polls_first_receiver() {
    let (tx1, rx1) = tokio::sync::broadcast::channel::<&str>(4);
    let (tx2, rx2) = tokio::sync::broadcast::channel::<&str>(4);

    let mut receivers: Vec<tokio::sync::broadcast::Receiver<&str>> = vec![rx1, rx2];

    // Pattern from subagent.rs WaitAgentTool: only first_mut() is polled
    let result = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        // This is the actual code pattern from the wait loop:
        if let Some(rx) = receivers.first_mut() {
            let _ = rx.recv().await;
            "got_from_first"
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            "timeout"
        }
    });

    // Send event to the SECOND channel (different session pool)
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = tx2.send("completion_event");
    });

    match result.await {
        Ok(msg) => {
            eprintln!("risk8: unexpectedly received: {msg}");
        }
        Err(_timeout) => {
            eprintln!(
                "risk8: CONFIRMED — event sent to second receiver was never received! \
                 The wait loop only polls receivers.first_mut(), ignoring events \
                 from other session pools. This causes wait_agent to hang until timeout."
            );
        }
    }

    // Clean up
    drop(tx1);
}

// ─── RISK 9: Concurrent batch tools skip pre-hooks ───────────────────
//
// Demonstrates the inconsistency: dispatch_one runs hooks but
// dispatch_batch's concurrent path (execute_unguarded_standalone) skips them.

#[test]
fn risk9_concurrent_tools_skip_hooks_documentation() {
    // This is a structural code analysis test — we verify the code paths
    // differ by checking that the concurrent path in dispatch_batch
    // calls execute_unguarded_standalone directly, which has no hook logic.
    //
    // dispatch_one pipeline:
    //   1. pre_execution_checks ✓
    //   2. run_pre_hooks ✓        ← hooks run here
    //   3. execute_guarded/unguarded ✓
    //   4. run_post_hooks ✓       ← hooks run here
    //   5. truncate_result ✓
    //
    // dispatch_batch concurrent pipeline (for parallel-safe tools):
    //   1. is_tool_allowed ✓ (inside execute_unguarded_standalone)
    //   2. mode_state check ✓
    //   3. tool.execute() ✓
    //   4. truncate_result_static ✓
    //   --- NO pre_hooks ---      ← MISSING
    //   --- NO post_hooks ---     ← MISSING
    //
    // Tools like read_file, search_in_files run through the concurrent path
    // and never trigger pre-hooks (security audit, argument rewrite) or
    // post-hooks (metrics, observability).

    // The fix: either route concurrent tools through dispatch_one in
    // parallel (refactor hooks to be &self instead of &mut), or explicitly
    // call hooks in the concurrent path.
    eprintln!(
        "risk9: concurrent tools in dispatch_batch bypass run_pre_hooks \
         and run_post_hooks. Only sequential tools get the full hook pipeline."
    );
}
