use std::io::Read;
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

    session.write_input(b"echo HELLO_PTY\n").unwrap();
    thread::sleep(Duration::from_millis(200));

    let mut reader = session.get_reader().unwrap();
    let mut buf = [0u8; 4096];
    let n = reader.read(&mut buf).unwrap();
    let output = String::from_utf8_lossy(&buf[..n]);
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
    let mut session = PtySession::spawn(
        "test-resize".to_string(),
        PtySessionConfig::default(),
    )
    .unwrap();

    assert_eq!(session.cols(), 80);
    assert_eq!(session.rows(), 24);

    session.resize(120, 40).unwrap();
    assert_eq!(session.cols(), 120);
    assert_eq!(session.rows(), 40);

    session.kill();
}
