use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

const BROADCAST_CAPACITY: usize = 256;

pub struct PtySessionConfig {
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub source: String,
}

impl Default for PtySessionConfig {
    fn default() -> Self {
        Self {
            shell: None,
            cwd: None,
            cols: 80,
            rows: 24,
            env: Vec::new(),
            source: "user".to_string(),
        }
    }
}

struct SubscriberGuard {
    count: Arc<AtomicUsize>,
    no_subscribers_since: Arc<Mutex<Option<Instant>>>,
}

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        if self.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.no_subscribers_since.lock() = Some(Instant::now());
        }
    }
}

/// Broadcast receiver that decrements the session subscriber count when dropped.
pub struct TrackedBroadcastReceiver {
    inner: broadcast::Receiver<Vec<u8>>,
    _guard: SubscriberGuard,
}

impl Deref for TrackedBroadcastReceiver {
    type Target = broadcast::Receiver<Vec<u8>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TrackedBroadcastReceiver {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

pub struct PtySession {
    pub id: String,
    pub source: String,
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    broadcast_tx: broadcast::Sender<Vec<u8>>,
    subscriber_count: Arc<AtomicUsize>,
    no_subscribers_since: Arc<Mutex<Option<Instant>>>,
    reader_shutdown: Arc<AtomicBool>,
    reader_handle: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
    pub created_at: Instant,
    pub last_activity: Arc<Mutex<Instant>>,
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.reader_shutdown.store(true, Ordering::SeqCst);
        {
            let mut child = self.child.lock();
            let _ = child.kill();
        }
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

impl PtySession {
    pub fn spawn(id: String, config: PtySessionConfig) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| format!("failed to open PTY: {e}"))?;

        let shell = config.shell.unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&shell);

        if let Some(ref cwd) = config.cwd {
            cmd.cwd(cwd);
        } else if let Ok(home) = std::env::var("HOME") {
            cmd.cwd(home);
        }

        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn shell: {e}"))?;

        let child = Arc::new(Mutex::new(child));

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take PTY writer: {e}"))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone PTY reader: {e}"))?;

        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);

        let reader_shutdown = Arc::new(AtomicBool::new(false));
        let tx_clone = broadcast_tx.clone();
        let child_for_reader = Arc::clone(&child);
        let shutdown_for_reader = Arc::clone(&reader_shutdown);
        let reader_handle = std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                if shutdown_for_reader.load(Ordering::SeqCst) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx_clone.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let mut child = child_for_reader.lock();
            let _ = child.wait();
        });

        let now = Instant::now();
        Ok(Self {
            id,
            source: config.source,
            master: pair.master,
            writer: Arc::new(Mutex::new(writer)),
            child,
            broadcast_tx,
            subscriber_count: Arc::new(AtomicUsize::new(0)),
            no_subscribers_since: Arc::new(Mutex::new(None)),
            reader_shutdown,
            reader_handle: Some(reader_handle),
            cols: config.cols,
            rows: config.rows,
            created_at: now,
            last_activity: Arc::new(Mutex::new(now)),
        })
    }

    pub fn subscribe(&self) -> TrackedBroadcastReceiver {
        self.subscriber_count.fetch_add(1, Ordering::SeqCst);
        *self.no_subscribers_since.lock() = None;
        TrackedBroadcastReceiver {
            inner: self.broadcast_tx.subscribe(),
            _guard: SubscriberGuard {
                count: Arc::clone(&self.subscriber_count),
                no_subscribers_since: Arc::clone(&self.no_subscribers_since),
            },
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscriber_count.load(Ordering::SeqCst)
    }

    pub fn no_subscribers_since(&self) -> Option<Instant> {
        *self.no_subscribers_since.lock()
    }

    pub fn write_input(&self, data: &[u8]) -> Result<(), String> {
        *self.last_activity.lock() = Instant::now();
        let mut writer = self.writer.lock();
        writer
            .write_all(data)
            .map_err(|e| format!("PTY write error: {e}"))
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        *self.last_activity.lock() = Instant::now();
        self.cols = cols;
        self.rows = rows;
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("PTY resize error: {e}"))
    }

    pub fn is_alive(&self) -> bool {
        let mut child = self.child.lock();
        child.try_wait().ok().flatten().is_none()
    }

    pub fn exit_code(&self) -> Option<u32> {
        let mut child = self.child.lock();
        child.try_wait().ok().flatten().map(|s| s.exit_code())
    }

    pub fn kill(&self) {
        let mut child = self.child.lock();
        let _ = child.kill();
        let _ = child.wait();
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn touch(&self) {
        *self.last_activity.lock() = Instant::now();
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().process_id()
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "windows") {
            "powershell.exe".to_string()
        } else {
            "/bin/bash".to_string()
        }
    })
}
