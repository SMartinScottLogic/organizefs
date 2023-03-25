use fuse_mt::mount;
use std::{env, ffi::OsStr};

fn main() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();

    let fuse_args = [
        OsStr::new("-o"),
        OsStr::new("fsname=organizefs"),
        OsStr::new("-o"),
        OsStr::new("auto_unmount"),
    ];

    let fs = organizefs::OrganizeFS::new(&args[1]);
    mount(fuse_mt::FuseMT::new(fs, 1), &args[2], &fuse_args[..]).unwrap();
}
