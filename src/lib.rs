use fuse_mt::{FilesystemMT, RequestInfo, ResultEmpty};
use tracing::debug;

#[derive(Debug, Default)]
pub struct OrganizeFS {}

impl OrganizeFS {
    pub fn new(root: &str) -> Self {
        tracing::info!(a = 123, startup = root, "OrganizeFS based on {root}");
        Self {}
    }
}

impl FilesystemMT for OrganizeFS {
    fn init(&self, _req: RequestInfo) -> ResultEmpty {
        debug!("init");
        Ok(())
    }

    fn destroy(&self) {
        debug!("destroy");
    }
}
