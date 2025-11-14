use lazy_static::lazy_static;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::str::FromStr;
use std::sync::RwLock;

pub struct Config {
    pub port: u16,
    pub listen_addr: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8080,
            listen_addr: "127.0.0.1".into(),
        }
    }
}

lazy_static! {
    pub static ref CONFIG: RwLock<Config> = RwLock::new(Config::default());
}

fn read_value(key: &str, value: &str, config: &mut Config) {
    match key {
        "Port" => config.port = u16::from_str_radix(value, 10).unwrap_or(8080),
        "Listen" => config.listen_addr = String::from_str(value).unwrap_or("127.0.0.1".into()),
        _ => (),
    }
}

fn read_line(line: &str, config: &mut Config) {
    let trim = line.trim();

    // Ignore comment line
    if trim.starts_with("#") {
        return;
    }

    // Find either space or tab separator(s) between key and value
    // ie. Port     443
    let mut index = 0;
    match trim.find(' ') {
        Some(i) => index = i,
        _ => match trim.find('\t') {
            Some(i) => index = i,
            _ => (),
        },
    }

    // Couldn't find any space or tab seperators
    if index == 0 {
        return;
    }

    let s = trim.split_at(index);
    let key = s.0.trim();
    let value = s.1.trim();
    // println!("key: '{}' value = '{}'", key, value);

    read_value(key, value, config);
}

pub fn load(file_path: &str) -> Result<(), io::Error> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut config = CONFIG.write().unwrap();
    for line in reader.lines() {
        read_line(&line?, &mut config);
    }

    Ok(())
}
