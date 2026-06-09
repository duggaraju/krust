pub mod dispatch;
pub mod handlers;
pub mod impls;
pub mod numbers;

pub fn init() {
    log::info!("Initializing syscall entry point");
}
