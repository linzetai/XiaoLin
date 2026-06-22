use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio::time;

use crate::session::{PtySession, PtySessionConfig};

const DEFAULT_MAX_SESSIONS: usize = 8;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

pub struct PtySessionManager {
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
    max_sessions: usize,
    idle_timeout: Duration,
}

impl PtySessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            max_sessions: DEFAULT_MAX_SESSIONS,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }

    pub fn create_session(&self, config: PtySessionConfig) -> Result<String, String> {
        let mut sessions = self.sessions.lock();
        if sessions.len() >= self.max_sessions {
            drop(sessions);
            self.cleanup_idle_sessions();
            sessions = self.sessions.lock();
            if sessions.len() >= self.max_sessions {
                return Err("max sessions reached".into());
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let session = PtySession::spawn(id.clone(), config)?;
        tracing::info!(session_id = %id, "PTY session created");
        sessions.insert(id.clone(), session);
        Ok(id)
    }

    pub fn create_session_with_subscriber(
        &self,
        config: PtySessionConfig,
    ) -> Result<(String, broadcast::Receiver<Vec<u8>>), String> {
        let mut sessions = self.sessions.lock();
        if sessions.len() >= self.max_sessions {
            drop(sessions);
            self.cleanup_idle_sessions();
            sessions = self.sessions.lock();
            if sessions.len() >= self.max_sessions {
                return Err("max sessions reached".into());
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let session = PtySession::spawn(id.clone(), config)?;
        let rx = session.subscribe();
        tracing::info!(session_id = %id, "PTY session created with subscriber");
        sessions.insert(id.clone(), session);
        Ok((id, rx))
    }

    pub fn subscribe(&self, id: &str) -> Option<broadcast::Receiver<Vec<u8>>> {
        let sessions = self.sessions.lock();
        sessions.get(id).map(|s| s.subscribe())
    }

    pub fn get_session<F, R>(&self, id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&PtySession) -> R,
    {
        let sessions = self.sessions.lock();
        sessions.get(id).map(f)
    }

    pub fn with_session_mut<F, R>(&self, id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut PtySession) -> R,
    {
        let mut sessions = self.sessions.lock();
        sessions.get_mut(id).map(f)
    }

    pub fn close_session(&self, id: &str) -> bool {
        let mut sessions = self.sessions.lock();
        if let Some(session) = sessions.remove(id) {
            session.kill();
            tracing::info!(session_id = %id, "PTY session closed");
            true
        } else {
            false
        }
    }

    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock();
        sessions
            .values()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                source: s.source.clone(),
                alive: s.is_alive(),
                cols: s.cols(),
                rows: s.rows(),
                idle_secs: s.last_activity.lock().elapsed().as_secs(),
            })
            .collect()
    }

    pub fn count_by_source(&self, source: &str) -> usize {
        let sessions = self.sessions.lock();
        sessions.values().filter(|s| s.source == source).count()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().len()
    }

    pub fn start_cleanup_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = time::interval(CLEANUP_INTERVAL);
            loop {
                interval.tick().await;
                mgr.cleanup_idle_sessions();
            }
        })
    }

    fn cleanup_idle_sessions(&self) {
        let mut sessions = self.sessions.lock();
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for (id, session) in sessions.iter() {
            let idle = now.duration_since(*session.last_activity.lock());
            if idle > self.idle_timeout || !session.is_alive() {
                to_remove.push(id.clone());
            }
        }

        for id in &to_remove {
            if let Some(session) = sessions.remove(id) {
                session.kill();
                tracing::info!(session_id = %id, "PTY session cleaned up (idle/dead)");
            }
        }
    }
}

impl Default for PtySessionManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SessionInfo {
    pub id: String,
    pub source: String,
    pub alive: bool,
    pub cols: u16,
    pub rows: u16,
    pub idle_secs: u64,
}
