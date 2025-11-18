use ipnet::IpNet;
use lazy_static::lazy_static;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::net::IpAddr;
use std::str::FromStr;
use tokio::sync::RwLock;
use trust_dns_resolver::config::ResolverConfig;

const DEFAULT_ADDR: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;
const MAX_IDLE_TIMEOUT: u16 = 600;

pub struct Config {
    pub port: u16,
    pub listen_addr: String,
    pub idle_timeout: u16,
    allow_list: Vec<IpAddr>,
    deny_list: Vec<IpAddr>,
    pub doh_resolver: Option<ResolverConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            listen_addr: DEFAULT_ADDR.into(),
            idle_timeout: MAX_IDLE_TIMEOUT,
            allow_list: vec![],
            deny_list: vec![],
            doh_resolver: None,
        }
    }
}

impl Config {
    pub fn is_allowed(&self, ip: &IpAddr) -> bool {
        (self.allow_list.is_empty() || self.allow_list.contains(&ip))
            && (self.deny_list.is_empty() || !self.deny_list.contains(&ip))
    }
}

lazy_static! {
    pub static ref CONFIG: RwLock<Config> = RwLock::new(Config::default());
}

fn read_value(key: &str, value: &str, config: &mut Config) {
    match key {
        "Port" => config.port = u16::from_str_radix(value, 10).unwrap_or(DEFAULT_PORT),
        "Listen" => config.listen_addr = String::from_str(value).unwrap_or(DEFAULT_ADDR.into()),
        "Timeout" => {
            config.idle_timeout = u16::from_str_radix(value, 10).unwrap_or(MAX_IDLE_TIMEOUT);
            if config.idle_timeout > MAX_IDLE_TIMEOUT {
                config.idle_timeout = MAX_IDLE_TIMEOUT;
            }
        }
        "Allow" => {
            let mut ipstr = format! {"{value}"};
            if None == value.find('/') {
                ipstr += &format!("/32");
            }
            if let Ok(ip) = IpNet::from_str(&ipstr) {
                for host in ip.hosts() {
                    if !config.allow_list.contains(&host) {
                        config.allow_list.push(host);
                    }
                }
            } else {
                println!("{value} parse error");
            }
        }
        "Deny" => {
            let mut ipstr = format! {"{value}"};
            if None == value.find('/') {
                ipstr += &format!("/32");
            }
            if let Ok(ip) = IpNet::from_str(&ipstr) {
                for host in ip.hosts() {
                    if !config.deny_list.contains(&host) {
                        config.deny_list.push(host);
                    }
                }
            } else {
                println!("{value} parse error");
            }
        }
        // Additional configs
        "DoHResolver" => {
            config.doh_resolver = match value {
                "cloudfare" => Some(ResolverConfig::cloudflare_tls()),
                "google" => Some(ResolverConfig::google_tls()),
                "quad9" => Some(ResolverConfig::quad9_tls()),
                _ => None,
            }
        }
        _ => (),
    }
}

//  TODO: Read double quoted value that may contain spaces
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

pub async fn load(file_path: &str) -> Result<(), io::Error> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut config = CONFIG.write().await;
    for line in reader.lines() {
        read_line(&line?, &mut config);
    }

    Ok(())
}
