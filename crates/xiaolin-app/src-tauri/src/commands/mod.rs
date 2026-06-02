pub mod agent;
pub mod clipboard;
pub mod config;
pub mod migration;
pub mod session;
pub mod skill;
pub mod voice;

// Only export IPC commands for local file operations
// All business logic (chat, sessions CRUD, agents, etc.) goes through WebSocket
pub use agent::{read_identity_files, upload_agent_avatar};
pub use clipboard::{
    clipboard_read_image, clipboard_read_text, clipboard_write_image, clipboard_write_text,
    read_image_file,
};
pub use config::get_gateway_info;
pub use migration::{export_data, import_data};
pub use session::export_session_content;
pub use skill::upload_skill;
pub use voice::{stt_available, transcribe_audio};