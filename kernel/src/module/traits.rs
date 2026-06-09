#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleError {
    InitFailed,
    CleanupFailed,
    DependencyMissing(&'static str),
    AlreadyLoaded,
    NotLoaded,
    InvalidModule,
}

pub trait KernelModule: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn init(&self) -> Result<(), ModuleError>;
    fn cleanup(&self) -> Result<(), ModuleError>;

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
