pub mod dispatch;
pub mod handlers;
pub mod impls;
pub mod numbers;

pub fn init(trace_enabled: bool) {
    dispatch::set_trace_enabled(trace_enabled);
    log::info!("Initializing syscall entry point");
    if trace_enabled {
        log::info!("syscall tracing enabled");
    }
}

pub fn set_trace_enabled(enabled: bool) {
    dispatch::set_trace_enabled(enabled);
}
