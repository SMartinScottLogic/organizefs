use fuse_mt::spawn_mount;

fn main() {
    println!("Hello, world!");

    let fs = OrganizeFS::new();
    spawn_mount(fs, "", &[]);
}
