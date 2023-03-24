use fuse_mt::spawn_mount;

fn main() {
    println!("Hello, world!");

    let fuse_args = [OsStr::new("-o"), OsStr::new("fsname=organizefs")];

    let fs = organizefs::OrganizeFS::new();
    spawn_mount(fuse_mt::FuseMT::new(fs, 1), "", &fuse_args[..]);
}
