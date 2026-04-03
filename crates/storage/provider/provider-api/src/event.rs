use katana_primitives::block::BlockNumber;
use katana_rpc_types::event::{EventFilter, GetEventsResponse};

use crate::ProviderResult;

/// Upstream event proxy for forked providers.
///
/// When a Katana node is running in fork mode, event queries that span
/// pre-fork blocks must be proxied to the upstream RPC rather than
/// iterated per-block locally (which would require N lazy-fetch RPCs).
///
/// This trait provides the fork boundary and a bulk event proxy.
/// The RPC layer uses this to split cross-fork event queries:
/// - pre-fork range → `upstream_events()` (single upstream RPC call)
/// - post-fork range → local iteration via receipts (existing path)
///
/// Non-forked providers (e.g. `DbProviderFactory`) return `None` from
/// `fork_block()`, and the RPC layer skips the proxy path entirely.
pub trait UpstreamEventProxy: Send + Sync {
    /// Returns the fork point block number, or `None` if not forked.
    fn fork_block(&self) -> Option<BlockNumber> {
        None
    }

    /// Fetch events from the upstream provider for pre-fork blocks.
    ///
    /// The filter's block range MUST be entirely at or below the fork point.
    /// Called only when `fork_block()` returns `Some`.
    fn upstream_events(
        &self,
        filter: EventFilter,
        continuation_token: Option<String>,
        chunk_size: u64,
    ) -> ProviderResult<GetEventsResponse> {
        let _ = (filter, continuation_token, chunk_size);
        Err(crate::ProviderError::Other("upstream_events not available on non-forked provider".into()))
    }
}
