use std::ops::Index;

use common::FsFile;
use file_proc_macro::FsFile;

#[derive(FsFile)]
struct One {
    #[fsfile = "meta"]
    one_meta: String,
    #[fsfile = "size"]
    one_size: String,
    _data: String,
}

#[derive(FsFile)]
struct Two<'a> {
    #[fsfile = "meta"]
    #[fsfile = "size"]
    data: &'a str,
}

fn main() -> std::io::Result<()> {
    println!("test");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_main() {
        assert!(main().is_ok());
    }

    #[test]
    fn one() {
        let one = One {
            one_meta: "m".into(),
            one_size: "s".into(),
            _data: "d".into(),
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
