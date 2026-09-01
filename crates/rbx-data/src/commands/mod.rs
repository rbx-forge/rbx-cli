//! One module per family of subcommand, and the context they share.
//!
//! `run` in `lib.rs` resolves the store, the universe and the credentials once,
//! then hands the whole command to the module that owns it. The split is by
//! what a subcommand acts on rather than by how it is implemented: a container,
//! one entry, or nothing at all.
//!
//! Each module keeps its own `match`, so adding a subcommand touches the
//! grouping here and one module, and the compiler names the second for you: a
//! variant that is routed to the wrong family reaches an `unreachable!` that
//! says so.

pub(crate) mod entry;
pub(crate) mod inspect;
pub(crate) mod store;

use rbx_core::GlobalFlags;

use crate::api::Api;
use crate::cli::Command;
use crate::json::Store;

/// What every subcommand below needs, resolved once before any of them runs.
///
/// Held by reference rather than rebuilt per subcommand because resolving it
/// involves reading `rbxplace.toml` and building an HTTP client, and because a
/// second resolution could disagree with the first.
pub(crate) struct Ctx<'a> {
    pub(crate) api: Api,
    pub(crate) store: Store,
    pub(crate) universe_id: u64,
    pub(crate) global: &'a GlobalFlags,
}

/// Which module owns a subcommand.
///
/// A function rather than a `match` inline in `run`, because the classification
/// has to happen while `command` is only borrowed and the call that follows
/// moves it.
pub(crate) enum Group {
    Store,
    Entry,
    Inspect,
}

pub(crate) fn group(command: &Command) -> Group {
    match command {
        Command::Snapshot { .. }
        | Command::Stores { .. }
        | Command::DeleteStore { .. }
        | Command::RestoreStore { .. } => Group::Store,

        Command::Get { .. }
        | Command::Set { .. }
        | Command::Reset { .. }
        | Command::DeleteKey { .. }
        | Command::RestoreKey { .. }
        | Command::Copy { .. }
        | Command::Increment { .. } => Group::Entry,

        Command::List { .. } | Command::Revisions { .. } | Command::Diff { .. } => Group::Inspect,

        // Answered by `run` before this is reached: it needs the credentials
        // and the raw store name rather than the resolved context, so it is
        // routed before one is built.
        Command::Ordered { .. } => Group::Inspect,
    }
}

impl Ctx<'_> {
    /// An [`Api`] pointed at another universe, with the same store, scope and
    /// credentials.
    ///
    /// `copy` and `diff` reach across environments, so they need a second
    /// client for the same store name in a different universe. Built from this
    /// one rather than resolved again: a second resolution could disagree with
    /// the first, and the store and scope are the command line's, not the
    /// universe's.
    pub(crate) fn build(&self, universe_id: u64) -> Api {
        Api {
            client: rbx_core::api::build_client(),
            base: self.api.base.clone(),
            api_key: self.api.api_key.clone(),
            universe_id,
            datastore: self.api.datastore.clone(),
            scope: self.api.scope.clone(),
        }
    }
}
