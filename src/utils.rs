use std::fs::File;
use std::io::{self, Write};

pub fn create_pidfile(path: &str) -> io::Result<()> {
    let mut f = File::options()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let s = std::process::id().to_string() + "\n".into();
    f.write_all(s.as_bytes())
}
