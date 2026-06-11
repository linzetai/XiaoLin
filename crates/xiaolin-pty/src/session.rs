use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

pub struct PtySessionConfig {
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
}

impl Default for PtySessionConfig {
    fn default() -> Self {
        Self {
            shell: None,
            cwd: None,
            cols: 80,
            rows: 24,
            env: Vec::new(),
        }
    }
}

pub struct PtySession {
    pub id: String,
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    cols: u16,
    rows: u16,
    pub created_at: Instant,
    pub last_activity: Arc<Mutex<Instant>>,
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

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take PTY writer: {e}"))?;

        let now = Instant::now();
        Ok(Self {
            id,
            master: pair.master,
            writer: Arc::new(Mutex::new(writer)),
            child: Arc::new(Mutex::new(child)),
            cols: config.cols,
            rows: config.rows,
            created_at: now,
            last_activity: Arc::new(Mutex::new(now)),
        })
    }

    pub fn write_input(&self, data: &[u8]) -> Result<(), String> {
        *self.last_activity.lock() = Instant::now();
        let mut writer = self.writer.lock();
        writer
            .write_all(data)
            .map_err(|e| format!("PTY write error: {e}"))
    }

    pub fn get_reader(&self) -> Result<Box<dyn Read + Send>, String> {
        self.master
            .try_clone_reader()
            .map_err(|e| format!("clone reader: {e}"))
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
