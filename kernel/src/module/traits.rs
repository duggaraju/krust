use alloc::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleError {
    InitFailed,
    CleanupFailed,
    DependencyMissing(&'static str),
    AlreadyLoaded,
    NotLoaded,
    InvalidModule,
}

pub trait KernelRegistry: Send + Sync {
    fn register_module(&self, module: Arc<dyn KernelModule>) -> Result<(), ModuleError>;

    fn unregister_module(&self, name: &str) -> Result<(), ModuleError>;

    fn register_filesystem(
        &self,
        fs: Arc<dyn crate::fs::vfs::FileSystem>,
    ) -> Result<(), ModuleError>;

    fn unregister_filesystem(&self, name: &str) -> Result<(), ModuleError>;

    #[cfg(feature = "drivers")]
    fn register_device(
        &self,
        name: &str,
        device: Arc<dyn crate::drivers::traits::Device>,
        major: u16,
        minor: u16,
    ) -> Result<(), ModuleError>;

    #[cfg(feature = "drivers")]
    fn unregister_device(&self, name: &str) -> Result<(), ModuleError>;
}

pub trait KernelModule: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn description(&self) -> &str {
        ""
    }
    fn init(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError>;
    fn cleanup(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError>;

    fn dependencies(&self) -> &[&str] {
        &[]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub author: &'static str,
    pub description: &'static str,
    pub license: &'static str,
}
