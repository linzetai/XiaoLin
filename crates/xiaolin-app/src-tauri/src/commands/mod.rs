pub mod agent;
pub mod audio_capture;
pub mod browser;
pub mod clipboard;
pub mod config;
pub mod file_viewer;
pub mod http_proxy;
pub mod migration;
pub mod session;
pub mod skill;
pub mod voice;

// Only export IPC commands for local file operations
// All business logic (chat, sessions CRUD, agents, etc.) goes through WebSocket
pub use agent::{read_identity_files, upload_agent_avatar};
pub use clipboard::{
    clipboard_read_image, clipboard_read_text, clipboard_write_image, clipboard_write_text,
    read_image_file, ClipboardState,
};
pub use config::get_gateway_info;
pub use file_viewer::{list_directory, read_binary_for_viewer, read_file_for_viewer};
pub use http_proxy::http_proxy;
pub use migration::{export_data, import_data};
pub use session::export_session_content;
pub use skill::upload_skill;
pub use voice::{stt_available, transcribe_audio};
pub use audio_capture::{
    native_audio_available, start_native_recording, stop_native_recording, AudioCaptureState,
};
pub use browser::{
    browser_close_page, browser_eval_js, browser_go_back, browser_go_forward, browser_hide_all_pages,
    browser_list_pages, browser_navigate, browser_open_page, browser_reload, browser_resize_webview,
    browser_show_page,
};