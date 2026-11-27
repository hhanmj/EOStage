mod asf_cached_search;
mod cancellation;
mod dispatcher;
mod protocol_io;
mod store_dispatch;

pub use cancellation::{CancellationRegistry, CatalogueRequestLease, CatalogueRequestRegistry};
pub use dispatcher::{
    catalogue_route_streams, is_streaming_cap_order, retry_whole_response_streamed,
    CatalogSearchForApplication, CatalogSearchNoopSink, CatalogSearchStreamOutcome,
    CatalogSearchStreamSink, CatalogSearchStreamedOutcome, CatalogSourceAttempt,
    CatalogSourceAttemptOutcome, CatalogSourceFallbackReason, DispatchClass, Dispatcher,
    LiveAsfSearch, LiveCatalogSearch, ProductionCatalogRouter,
    RegisteredCatalogSearchForApplication, RequestDispatcher,
};
pub use protocol_io::{
    serve, serve_with_limits, DEFAULT_WORKER_COUNT, DEFAULT_WORK_QUEUE_CAPACITY, MAX_MESSAGE_BYTES,
};

pub fn run_stdio() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let dispatcher = Dispatcher::try_new()
        .map_err(|_| std::io::Error::other("sidecar provider initialization failed"))?;
    serve(stdin.lock(), stdout, &dispatcher)
}

pub fn run_stdio_with_store_path(path: impl Into<std::path::PathBuf>) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let dispatcher = Dispatcher::try_new_with_store_path(path.into())
        .map_err(|_| std::io::Error::other("sidecar provider initialization failed"))?;
    serve(stdin.lock(), stdout, &dispatcher)
}

pub fn run_stdio_with_downloads(path: impl Into<std::path::PathBuf>) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let dispatcher = Dispatcher::try_new_with_downloads(path.into())
        .map_err(|_| std::io::Error::other("sidecar download initialization failed"))?;
    serve(stdin.lock(), stdout, &dispatcher)
}
