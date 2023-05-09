use std::ops::Index;

use file_proc_macro::FsFile;
use organizefs::common::FsFile;

#[derive(Default, FsFile)]
struct One {
    #[fsfile = "meta"]
    one_meta: String,
    #[fsfile = "size"]
    one_size: String,
    data: String,
}

#[derive(Default, FsFile)]
struct Two<'a> {
    #[fsfile = "meta"]
    #[fsfile = "size"]
    data: &'a str,
}

fn main() {
    println!("test");
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn one() {
        let one = One {
            one_meta: "m".into(),
            one_size: "s".into(),
            data: "d".into(),
        };
        assert_eq!(&one["meta"], "m");
        assert_eq!(&one["size"], "s");
    }

    #[test]
    fn two() {
        let two = Two { data: "joint" };
        assert_eq!(&two["meta"], "joint");
        assert_eq!(&two["size"], "joint");
    }
}
