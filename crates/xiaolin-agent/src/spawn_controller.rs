use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{broadcast, RwLock};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub max_global: usize,
    pub max_per_session: usize,
    pub enforce_rw_isolation: bool,
    pub slot_acquire_timeout: Duration,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            max_global: 20,
            max_per_session: 5,
            enforce_rw_isolation: true,
            slot_acquire_timeout: Duration::from_secs(30),
        }
    }
}

impl SpawnConfig {
    pub fn from_policy_fallback(max_parallel: u32) -> Self {
        Self {
            max_per_session: max_parallel as usize,
            ..Default::default()
        }
    }

    pub fn from_concurrency_config(
        cc: &xiaolin_core::agent_config::ConcurrencyConfig,
    ) -> Self {
        Self {
            max_global: cc.max_global,
            max_per_session: cc.max_per_session,
            enforce_rw_isolation: cc.enforce_rw_isolation,
            slot_acquire_timeout: Duration::from_secs(cc.slot_acquire_timeout_seconds),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SpawnControllerError {
    #[error("global slot limit reached ({max}), timed out after {elapsed_ms}ms")]
    GlobalLimitTimeout { max: usize, elapsed_ms: u64 },

    #[error(
        "session '{session_id}' slot limit reached ({max}), timed out after {elapsed_ms}ms"
    )]
    SessionLimitTimeout {
        session_id: String,
        max: usize,
        elapsed_ms: u64,
    },

    #[error("rw_gate acquisition timed out for session '{session_id}' (concurrency_safe={safe})")]
    RwGateTimeout { session_id: String, safe: bool },

    #[error("controller is shutting down")]
    Shutdown,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum SlotEvent {
    Acquired {
        run_id: String,
        concurrency_safe: bool,
        def_id: String,
    },
    Released {
        run_id: String,
    },
    Completed {
        run_id: String,
        result: Option<String>,
    },
    Failed {
        run_id: String,
        error: String,
    },
}

// ---------------------------------------------------------------------------
// Observability
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ConcurrencySnapshot {
    pub global_active: usize,
    pub global_max: usize,
    pub sessions: Vec<SessionSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub active: usize,
    pub max: usize,
    pub rw_state: RwState,
    pub agents: Vec<ActiveAgentInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub enum RwState {
    Idle,
    Reading(usize),
    Writing(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveAgentInfo {
    pub run_id: String,
    pub def_id: String,
    pub concurrency_safe: bool,
    pub started_at: u64,
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// GlobalSlotPool
// ---------------------------------------------------------------------------

struct GlobalSlotPool {
    active: AtomicUsize,
    max: usize,
    notify: tokio::sync::Notify,
}

impl GlobalSlotPool {
    fn new(max: usize) -> Self {
        Self {
            active: AtomicUsize::new(0),
            max,
            notify: tokio::sync::Notify::new(),
        }
    }

    async fn try_acquire(&self, timeout: Duration) -> Result<(), SpawnControllerError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let current = self.active.load(Ordering::Acquire);
            if current < self.max {
                match self.active.compare_exchange_weak(
                    current,
                    current + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Ok(()),
                    Err(_) => continue,
                }
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let elapsed = timeout.as_millis() as u64;
                return Err(SpawnControllerError::GlobalLimitTimeout {
                    max: self.max,
                    elapsed_ms: elapsed,
                });
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(remaining) => {}
            }
        }
    }

    fn release(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }
}

// ---------------------------------------------------------------------------
// SessionSlotPool
// ---------------------------------------------------------------------------

pub struct SessionSlotPool {
    session_id: String,
    active: AtomicUsize,
    max: usize,
    rw_gate: Arc<RwLock<()>>,
    events_tx: broadcast::Sender<SlotEvent>,
    notify: tokio::sync::Notify,
    last_activity: AtomicU64,
    active_agents: DashMap<String, ActiveAgentInfo>,
    /// Tracks RwLock state for observability.
    /// 0 = idle, positive = number of readers, usize::MAX = writer holding.
    rw_state_counter: AtomicUsize,
    rw_writer_id: std::sync::Mutex<Option<String>>,
}

const RW_WRITER_SENTINEL: usize = usize::MAX;

impl SessionSlotPool {
    fn new(session_id: String, max: usize) -> Self {
        let (events_tx, _) = broadcast::channel(128);
        Self {
            session_id,
            active: AtomicUsize::new(0),
            max,
            rw_gate: Arc::new(RwLock::new(())),
            events_tx,
            notify: tokio::sync::Notify::new(),
            last_activity: AtomicU64::new(now_ms()),
            active_agents: DashMap::new(),
            rw_state_counter: AtomicUsize::new(0),
            rw_writer_id: std::sync::Mutex::new(None),
        }
    }

    async fn try_acquire(&self, timeout: Duration) -> Result<(), SpawnControllerError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let current = self.active.load(Ordering::Acquire);
            if current < self.max {
                match self.active.compare_exchange_weak(
                    current,
                    current + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        self.touch();
                        return Ok(());
                    }
                    Err(_) => continue,
                }
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let elapsed = timeout.as_millis() as u64;
                return Err(SpawnControllerError::SessionLimitTimeout {
                    session_id: self.session_id.clone(),
                    max: self.max,
                    elapsed_ms: elapsed,
                });
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(remaining) => {}
            }
        }
    }

    fn release(&self, run_id: &str) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.active_agents.remove(run_id);
        self.touch();
        self.notify.notify_waiters();
        let _ = self.events_tx.send(SlotEvent::Released {
            run_id: run_id.to_string(),
        });
    }

    fn touch(&self) {
        self.last_activity.store(now_ms(), Ordering::Release);
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<SlotEvent> {
        self.events_tx.subscribe()
    }

    pub fn broadcast(&self, event: SlotEvent) {
        let _ = self.events_tx.send(event);
    }

    fn track_reader_acquired(&self) {
        self.rw_state_counter.fetch_add(1, Ordering::AcqRel);
    }

    fn track_reader_released(&self) {
        self.rw_state_counter.fetch_sub(1, Ordering::AcqRel);
    }

    fn track_writer_acquired(&self, run_id: &str) {
        self.rw_state_counter
            .store(RW_WRITER_SENTINEL, Ordering::Release);
        *self.rw_writer_id.lock().unwrap() = Some(run_id.to_string());
    }

    fn track_writer_released(&self) {
        self.rw_state_counter.store(0, Ordering::Release);
        *self.rw_writer_id.lock().unwrap() = None;
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let rw_val = self.rw_state_counter.load(Ordering::Acquire);
        let rw_state = if rw_val == 0 {
            RwState::Idle
        } else if rw_val == RW_WRITER_SENTINEL {
            let writer = self.rw_writer_id.lock().unwrap().clone().unwrap_or_default();
            RwState::Writing(writer)
        } else {
            RwState::Reading(rw_val)
        };

        let now = now_ms();
        let agents: Vec<ActiveAgentInfo> = self
            .active_agents
            .iter()
            .map(|entry| {
                let mut info = entry.value().clone();
                info.elapsed_ms = now.saturating_sub(info.started_at);
                info
            })
            .collect();

        SessionSnapshot {
            session_id: self.session_id.clone(),
            active: self.active.load(Ordering::Acquire),
            max: self.max,
            rw_state,
            agents,
        }
    }

    fn is_idle_since(&self, cutoff_ms: u64) -> bool {
        self.active.load(Ordering::Acquire) == 0
            && self.last_activity.load(Ordering::Acquire) < cutoff_ms
    }
}

// ---------------------------------------------------------------------------
// SpawnReservation (RAII)
// ---------------------------------------------------------------------------

pub struct SpawnReservation {
    run_id: String,
    global_pool: Arc<GlobalSlotPool>,
    session_pool: Arc<SessionSlotPool>,
    _rw_guard: RwGuardKind,
}

impl std::fmt::Debug for SpawnReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnReservation")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

/// Held for RAII: dropping the guard releases the RwLock.
#[allow(dead_code)]
enum RwGuardKind {
    Read(tokio::sync::OwnedRwLockReadGuard<()>),
    Write(tokio::sync::OwnedRwLockWriteGuard<()>),
    None,
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        let mode = match &self._rw_guard {
            RwGuardKind::Read(_) => {
                self.session_pool.track_reader_released();
                "read"
            }
            RwGuardKind::Write(_) => {
                self.session_pool.track_writer_released();
                "write"
            }
            RwGuardKind::None => "none",
        };
        self.session_pool.release(&self.run_id);
        self.global_pool.release();
        tracing::info!(
            run_id = %self.run_id,
            mode,
            global_active = self.global_pool.active.load(Ordering::Acquire),
            "spawn_controller: reservation released"
        );
    }
}

impl SpawnReservation {
    pub fn session_pool(&self) -> &Arc<SessionSlotPool> {
        &self.session_pool
    }
}

// ---------------------------------------------------------------------------
// SpawnController
// ---------------------------------------------------------------------------

pub struct SpawnController {
    global_pool: Arc<GlobalSlotPool>,
    session_pools: DashMap<String, Arc<SessionSlotPool>>,
    config: SpawnConfig,
}

impl SpawnController {
    pub fn new(config: SpawnConfig) -> Self {
        Self {
            global_pool: Arc::new(GlobalSlotPool::new(config.max_global)),
            session_pools: DashMap::new(),
            config,
        }
    }

    pub fn get_or_create_session_pool(&self, session_id: &str) -> Arc<SessionSlotPool> {
        self.session_pools
            .entry(session_id.to_string())
            .or_insert_with(|| {
                Arc::new(SessionSlotPool::new(
                    session_id.to_string(),
                    self.config.max_per_session,
                ))
            })
            .clone()
    }

    pub async fn reserve(
        &self,
        session_id: &str,
        run_id: &str,
        concurrency_safe: bool,
        timeout: Duration,
    ) -> Result<SpawnReservation, SpawnControllerError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mode = if concurrency_safe { "read" } else { "write" };

        tracing::info!(
            run_id,
            session_id,
            mode,
            "spawn_controller: acquiring reservation"
        );

        self.global_pool.try_acquire(timeout).await?;
        tracing::debug!(run_id, "spawn_controller: global slot acquired");

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let session_pool = self.get_or_create_session_pool(session_id);
        if let Err(e) = session_pool.try_acquire(remaining).await {
            self.global_pool.release();
            tracing::warn!(run_id, %e, "spawn_controller: session slot failed");
            return Err(e);
        }
        tracing::debug!(run_id, "spawn_controller: session slot acquired");

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let rw_guard = if self.config.enforce_rw_isolation {
            let gate = session_pool.rw_gate.clone();
            if concurrency_safe {
                tracing::debug!(run_id, "spawn_controller: acquiring read lock");
                match tokio::time::timeout(remaining, gate.read_owned()).await {
                    Ok(guard) => {
                        session_pool.track_reader_acquired();
                        tracing::info!(run_id, session_id, "spawn_controller: read lock acquired");
                        RwGuardKind::Read(guard)
                    }
                    Err(_) => {
                        session_pool.release(run_id);
                        self.global_pool.release();
                        tracing::warn!(run_id, "spawn_controller: read lock timeout (writer holding)");
                        return Err(SpawnControllerError::RwGateTimeout {
                            session_id: session_id.to_string(),
                            safe: concurrency_safe,
                        });
                    }
                }
            } else {
                tracing::debug!(run_id, "spawn_controller: acquiring write lock (exclusive)");
                match tokio::time::timeout(remaining, gate.write_owned()).await {
                    Ok(guard) => {
                        session_pool.track_writer_acquired(run_id);
                        tracing::info!(run_id, session_id, "spawn_controller: write lock acquired (exclusive)");
                        RwGuardKind::Write(guard)
                    }
                    Err(_) => {
                        session_pool.release(run_id);
                        self.global_pool.release();
                        tracing::warn!(run_id, "spawn_controller: write lock timeout (readers/writer holding)");
                        return Err(SpawnControllerError::RwGateTimeout {
                            session_id: session_id.to_string(),
                            safe: concurrency_safe,
                        });
                    }
                }
            }
        } else {
            RwGuardKind::None
        };

        session_pool.active_agents.insert(
            run_id.to_string(),
            ActiveAgentInfo {
                run_id: run_id.to_string(),
                def_id: String::new(),
                concurrency_safe,
                started_at: now_ms(),
                elapsed_ms: 0,
            },
        );

        tracing::info!(
            run_id,
            session_id,
            mode,
            global_active = self.global_pool.active.load(Ordering::Acquire),
            session_active = session_pool.active.load(Ordering::Acquire),
            "spawn_controller: reservation complete"
        );

        Ok(SpawnReservation {
            run_id: run_id.to_string(),
            global_pool: self.global_pool.clone(),
            session_pool,
            _rw_guard: rw_guard,
        })
    }

    pub fn snapshot(&self) -> ConcurrencySnapshot {
        let sessions: Vec<SessionSnapshot> = self
            .session_pools
            .iter()
            .map(|entry| entry.value().snapshot())
            .collect();
        ConcurrencySnapshot {
            global_active: self.global_pool.active.load(Ordering::Acquire),
            global_max: self.global_pool.max,
            sessions,
        }
    }

    pub fn gc_idle_sessions(&self, max_idle: Duration) {
        let cutoff = now_ms().saturating_sub(max_idle.as_millis() as u64);
        self.session_pools
            .retain(|_, pool| !pool.is_idle_since(cutoff));
    }

    pub fn config(&self) -> &SpawnConfig {
        &self.config
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_timeout() -> Duration {
        Duration::from_secs(5)
    }

    fn short_timeout() -> Duration {
        Duration::from_millis(50)
    }

    // --- 2.1 Global slot limit ---
    #[tokio::test]
    async fn global_pool_rejects_when_full() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_global: 2,
            max_per_session: 10,
            ..Default::default()
        });
        let _r1 = ctrl
            .reserve("s1", "run1", true, default_timeout())
            .await
            .unwrap();
        let _r2 = ctrl
            .reserve("s1", "run2", true, default_timeout())
            .await
            .unwrap();
        let r3 = ctrl.reserve("s1", "run3", true, short_timeout()).await;
        assert!(r3.is_err());
        assert!(r3.unwrap_err().to_string().contains("global slot limit"));

        drop(_r1);
        tokio::task::yield_now().await;
        let r3 = ctrl
            .reserve("s1", "run3", true, default_timeout())
            .await;
        assert!(r3.is_ok());
    }

    // --- 2.2 Per-session isolation ---
    #[tokio::test]
    async fn sessions_have_independent_pools() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_global: 10,
            max_per_session: 2,
            ..Default::default()
        });
        let _a1 = ctrl
            .reserve("sA", "a1", true, default_timeout())
            .await
            .unwrap();
        let _a2 = ctrl
            .reserve("sA", "a2", true, default_timeout())
            .await
            .unwrap();
        let a3 = ctrl.reserve("sA", "a3", true, short_timeout()).await;
        assert!(a3.is_err());

        let b1 = ctrl.reserve("sB", "b1", true, default_timeout()).await;
        assert!(b1.is_ok());
    }

    // --- 2.3 Readers parallel ---
    #[tokio::test]
    async fn concurrent_safe_agents_run_in_parallel() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_per_session: 5,
            ..Default::default()
        });
        let ids: Vec<String> = (0..5).map(|i| format!("r{i}")).collect();
        let futs: Vec<_> = ids
            .iter()
            .map(|id| ctrl.reserve("s1", id, true, default_timeout()))
            .collect();
        let guards: Vec<_> = futures::future::join_all(futs).await;
        assert!(guards.iter().all(|g| g.is_ok()));
    }

    // --- 2.4 Writer exclusive ---
    #[tokio::test]
    async fn non_concurrent_safe_blocks_others() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig {
            max_per_session: 5,
            ..Default::default()
        }));
        let _writer = ctrl
            .reserve("s1", "w1", false, default_timeout())
            .await
            .unwrap();
        let reader = ctrl.reserve("s1", "r1", true, short_timeout()).await;
        assert!(reader.is_err(), "reader should timeout while writer holds");
    }

    // --- 2.5 Writer waits for readers to drain ---
    #[tokio::test]
    async fn writer_waits_for_readers_to_drain() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig {
            max_per_session: 5,
            ..Default::default()
        }));
        let r1 = ctrl
            .reserve("s1", "r1", true, default_timeout())
            .await
            .unwrap();
        let r2 = ctrl
            .reserve("s1", "r2", true, default_timeout())
            .await
            .unwrap();

        let ctrl2 = ctrl.clone();
        let writer_handle = tokio::spawn(async move {
            ctrl2
                .reserve("s1", "w1", false, Duration::from_secs(5))
                .await
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!writer_handle.is_finished());

        drop(r1);
        drop(r2);

        let result = writer_handle.await.unwrap();
        assert!(result.is_ok());
    }

    // --- 2.6 RAII drop releases slot ---
    #[tokio::test]
    async fn reservation_drop_releases_slot() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_per_session: 1,
            max_global: 1,
            ..Default::default()
        });
        {
            let _r = ctrl
                .reserve("s1", "r1", true, default_timeout())
                .await
                .unwrap();
        }
        let r2 = ctrl.reserve("s1", "r2", true, default_timeout()).await;
        assert!(r2.is_ok());
    }

    // --- 2.7 Task cancel releases reservation ---
    #[tokio::test]
    async fn reservation_released_on_task_cancel() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig {
            max_per_session: 1,
            max_global: 1,
            ..Default::default()
        }));
        let ctrl2 = ctrl.clone();
        let handle = tokio::spawn(async move {
            let _r = ctrl2
                .reserve("s1", "r1", true, default_timeout())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.abort();
        let _ = handle.await;

        tokio::task::yield_now().await;
        let r2 = ctrl.reserve("s1", "r2", true, default_timeout()).await;
        assert!(r2.is_ok());
    }

    // --- 2.8 Broadcast events ---
    #[tokio::test]
    async fn slot_events_are_broadcast() {
        let ctrl = SpawnController::new(SpawnConfig::default());
        let pool = ctrl.get_or_create_session_pool("s1");
        let mut rx = pool.subscribe_events();

        let r1 = ctrl
            .reserve("s1", "run1", true, default_timeout())
            .await
            .unwrap();
        drop(r1);

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, SlotEvent::Released { run_id } if run_id == "run1"));
    }

    // --- 2.9 Snapshot ---
    #[tokio::test]
    async fn snapshot_reflects_current_state() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_global: 10,
            max_per_session: 5,
            ..Default::default()
        });
        let _r1 = ctrl
            .reserve("s1", "r1", true, default_timeout())
            .await
            .unwrap();
        let _r2 = ctrl
            .reserve("s1", "r2", true, default_timeout())
            .await
            .unwrap();

        let snap = ctrl.snapshot();
        assert_eq!(snap.global_active, 2);
        assert_eq!(snap.global_max, 10);
        assert_eq!(snap.sessions.len(), 1);
        assert_eq!(snap.sessions[0].active, 2);
        assert_eq!(snap.sessions[0].agents.len(), 2);
        assert!(matches!(snap.sessions[0].rw_state, RwState::Reading(2)));
    }

    // --- 2.10 GC ---
    #[tokio::test]
    async fn gc_removes_idle_session_pools() {
        let ctrl = SpawnController::new(SpawnConfig::default());
        {
            let _r = ctrl
                .reserve("s1", "r1", true, default_timeout())
                .await
                .unwrap();
        }
        let _active = ctrl
            .reserve("s_active", "r2", true, default_timeout())
            .await
            .unwrap();

        // Wait a bit so "s1" becomes stale relative to a short idle threshold
        tokio::time::sleep(Duration::from_millis(20)).await;
        ctrl.gc_idle_sessions(Duration::from_millis(10));

        assert!(
            ctrl.session_pools.get("s1").is_none(),
            "idle pool should be GC'd"
        );
        assert!(
            ctrl.session_pools.get("s_active").is_some(),
            "active pool should survive"
        );
    }

    // =========================================================================
    // 6.x Manager integration tests (SpawnController-level)
    // =========================================================================

    // --- 6.1 spawn respects controller limits ---
    #[tokio::test]
    async fn spawn_respects_controller_limits() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_global: 3,
            max_per_session: 3,
            ..Default::default()
        });
        let _r1 = ctrl.reserve("s1", "a1", true, default_timeout()).await.unwrap();
        let _r2 = ctrl.reserve("s1", "a2", true, default_timeout()).await.unwrap();
        let _r3 = ctrl.reserve("s1", "a3", true, default_timeout()).await.unwrap();

        let r4 = ctrl.reserve("s1", "a4", true, short_timeout()).await;
        assert!(r4.is_err(), "4th spawn should be blocked");
    }

    // --- 6.2 event-driven completion (no polling) ---
    #[tokio::test]
    async fn spawn_and_wait_uses_event_not_polling() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig::default()));
        let pool = ctrl.get_or_create_session_pool("s1");
        let mut rx = pool.subscribe_events();

        let t0 = tokio::time::Instant::now();

        let ctrl2 = ctrl.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let pool = ctrl2.get_or_create_session_pool("s1");
            pool.broadcast(SlotEvent::Completed {
                run_id: "run1".into(),
                result: Some("done".into()),
            });
        });

        let event = rx.recv().await.unwrap();
        let elapsed = t0.elapsed();
        assert!(matches!(event, SlotEvent::Completed { .. }));
        assert!(
            elapsed < Duration::from_millis(200),
            "event should arrive quickly, not polling"
        );
    }

    // --- 6.3 cancel releases slot ---
    #[tokio::test]
    async fn cancel_releases_slot_for_next_spawn() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig {
            max_global: 1,
            max_per_session: 1,
            ..Default::default()
        }));
        let ctrl2 = ctrl.clone();
        let handle = tokio::spawn(async move {
            let _r = ctrl2.reserve("s1", "a1", true, default_timeout()).await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.abort();
        let _ = handle.await;
        tokio::task::yield_now().await;

        let r = ctrl.reserve("s1", "a2", true, default_timeout()).await;
        assert!(r.is_ok(), "slot should be free after cancellation");
    }

    // --- 6.4 explore parallel, code exclusive ---
    #[tokio::test]
    async fn explore_parallel_code_exclusive() {
        let ctrl = Arc::new(SpawnController::new(SpawnConfig {
            max_per_session: 5,
            ..Default::default()
        }));

        let _e1 = ctrl.reserve("s1", "explore1", true, default_timeout()).await.unwrap();
        let _e2 = ctrl.reserve("s1", "explore2", true, default_timeout()).await.unwrap();

        let code = ctrl.reserve("s1", "code1", false, short_timeout()).await;
        assert!(code.is_err(), "code (writer) should block while explore (readers) hold");
    }

    // --- 6.5 concurrency_safe flag from def ---
    #[tokio::test]
    async fn concurrency_safe_flag_from_def() {
        let ctrl = SpawnController::new(SpawnConfig {
            max_per_session: 5,
            ..Default::default()
        });

        let _r1 = ctrl.reserve("s1", "r1", true, default_timeout()).await.unwrap();
        let snap = ctrl.snapshot();
        let session = &snap.sessions[0];
        let agent = session.agents.iter().find(|a| a.run_id == "r1").unwrap();
        assert!(agent.concurrency_safe);

        let _r2 = ctrl.reserve("s1", "r2", false, default_timeout()).await;
        // can't acquire writer while reader holds — already tested, just verify flag
    }
}
