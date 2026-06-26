use std::thread;
use std::time::Duration;

use xiaolin_pty::{PtySession, PtySessionConfig, PtySessionManager};

#[test]
fn spawn_session_and_echo() {
    let session = PtySession::spawn(
        "test-1".to_string(),
        PtySessionConfig {
            cols: 80,
            rows: 24,
            ..Default::default()
        },
    )
    .expect("spawn failed");

    assert!(session.is_alive());

    let mut rx = session.subscribe();

    session.write_input(b"echo HELLO_PTY\n").unwrap();
    thread::sleep(Duration::from_millis(500));

    let mut output = String::new();
    while let Ok(data) = rx.try_recv() {
        output.push_str(&String::from_utf8_lossy(&data));
    }
    assert!(
        output.contains("HELLO_PTY"),
        "expected HELLO_PTY in output, got: {output}"
    );

    session.kill();
    thread::sleep(Duration::from_millis(100));
    assert!(!session.is_alive());
}

#[test]
fn session_manager_create_and_close() {
    let mgr = PtySessionManager::new();
    let id = mgr
        .create_session(PtySessionConfig::default())
        .expect("create failed");

    assert_eq!(mgr.session_count(), 1);

    let sessions = mgr.list_sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, id);
    assert!(sessions[0].alive);
    assert_eq!(sessions[0].source, "user");

    mgr.close_session(&id);
    assert_eq!(mgr.session_count(), 0);
}

#[test]
fn session_manager_max_sessions() {
    let mgr = PtySessionManager::new();
    let mut ids = Vec::new();

    for _ in 0..8 {
        let id = mgr
            .create_session(PtySessionConfig::default())
            .expect("create failed");
        ids.push(id);
    }

    let result = mgr.create_session(PtySessionConfig::default());
    assert!(result.is_err(), "should reject 9th session");

    for id in &ids {
        mgr.close_session(id);
    }
}

#[test]
fn session_resize() {
    let mut session =
        PtySession::spawn("test-resize".to_string(), PtySessionConfig::default()).unwrap();

    assert_eq!(session.cols(), 80);
    assert_eq!(session.rows(), 24);

    session.resize(120, 40).unwrap();
    assert_eq!(session.cols(), 120);
    assert_eq!(session.rows(), 40);

    session.kill();
}

#[test]
fn create_session_with_subscriber() {
    let mgr = PtySessionManager::new();
    let (id, mut rx) = mgr
        .create_session_with_subscriber(PtySessionConfig::default())
        .expect("create with subscriber failed");

    mgr.get_session(&id, |s| s.write_input(b"echo SUB_TEST\n"))
        .unwrap()
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    let mut output = String::new();
    while let Ok(data) = rx.try_recv() {
        output.push_str(&String::from_utf8_lossy(&data));
    }
    assert!(
        output.contains("SUB_TEST"),
        "expected SUB_TEST in output, got: {output}"
    );

    mgr.close_session(&id);
}

#[test]
fn count_by_source() {
    let mgr = PtySessionManager::new();

    let _id1 = mgr
        .create_session(PtySessionConfig {
            source: "agent".to_string(),
            ..Default::default()
        })
        .unwrap();
    let _id2 = mgr
        .create_session(PtySessionConfig {
            source: "agent".to_string(),
            ..Default::default()
        })
        .unwrap();
    let _id3 = mgr.create_session(PtySessionConfig::default()).unwrap();

    assert_eq!(mgr.count_by_source("agent"), 2);
    assert_eq!(mgr.count_by_source("user"), 1);

    mgr.close_session(&_id1);
    mgr.close_session(&_id2);
    mgr.close_session(&_id3);
}
