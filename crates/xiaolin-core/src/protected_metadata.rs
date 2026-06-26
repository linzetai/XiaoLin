//! Protected workspace metadata directory names shared across sandbox and security.

/// Path component names for repository / workspace metadata that must not be
/// modified by untrusted tools (e.g. `.git`, VCS dirs, agent state).
///
/// Keep this list in sync anywhere workspace write policies are enforced.
pub const PROTECTED_METADATA_PATH_NAMES: &[&str] = &[".git", ".hg", ".svn", ".agents", ".xiaolin"];
