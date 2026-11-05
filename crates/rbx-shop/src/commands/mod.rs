pub mod check;
pub mod codegen;
pub mod forget;
pub mod init;
pub mod list;
pub mod pull;
pub mod rename;
pub mod show;
pub mod sync;

/// Said when a listing from Roblox carries a resource with no id, which `init`
/// and `pull` then skip. Recording it as 0 would point every later sync at
/// nothing and leave the real one unmanaged.
pub(crate) fn warn_no_id(kind: crate::config::ResourceKind, name: &str) {
    eprintln!("warning: Roblox listed {kind} '{name}' with no id, so it was skipped.");
}
