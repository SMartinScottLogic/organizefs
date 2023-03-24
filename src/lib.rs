use fuse_mt::FilesystemMT;

#[derive(Debug, Default)]
pub struct OrganizeFS {

}

impl OrganizeFS {
    pub fn new() -> Self {
        Self {}
    }
}

impl FilesystemMT for OrganizeFS {}
