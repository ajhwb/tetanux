use base64::Engine;
use clap::Parser;
use socket2::SockRef;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::os::fd::AsFd;
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::oneshot;
use url::Url;

mod cli;
use cli::Cli;
mod config;
use config::CONFIG;
mod resolver;
use resolver::resolve;
mod utils;

async fn relay(id: &str, reader: &mut OwnedReadHalf, writer: &mut OwnedWriteHalf) {
    let mut buf = vec![0; 8 * 1024];
    let config = CONFIG.read().await;
    let idle_timeout: u64 = config.idle_timeout.into();

    loop {
        let mut nread: usize = 0;
        let mut is_end = false;

        if let Err(_) = tokio::time::timeout(Duration::from_secs(idle_timeout), async {
            match reader.read(&mut buf).await {
                Ok(n) => {
                    if n > 0 {
                        nread = n;
                    } else {
                        // EOF
                        is_end = true;
                    }
                }
                Err(e) => {
                    eprintln!("{id}: read error: {}", e.to_string());
                    is_end = true;
                }
            }
        })
        .await
        {
            eprintln!("task {id} idle timeout");
            break;
        }

        if is_end {
            break;
        }

        if let Err(e) = writer.write_all(&buf[..nread]).await {
            eprintln!("{id}: write error: {}", e.to_string());
            break;
        }
    }
}

async fn tunnel(mut client: TcpStream, path: &str) -> Result<(), std::io::Error> {
    let tunnel_id = tokio::task::id();
    eprintln!("tunnel[{tunnel_id}] start");
    let stream: TcpStream;
    let config = CONFIG.read().await;

    if let Some(r) = &config.doh_resolver {
        let name: &str;
        let port: u16;
        match path.find(':') {
            Some(i) => {
                name = &path[..i];
                port = u16::from_str_radix(&path[i + 1..], 10).unwrap_or(443);
            }
            None => {
                let err = Error::new(ErrorKind::InvalidData, "Invalid path");
                return Err(err);
            }
        };

        match resolve(r.clone(), &name, port).await {
            Ok(v) => stream = TcpStream::connect(&v[..]).await?,
            Err(e) => {
                let err = Error::new(ErrorKind::AddrNotAvailable, e.to_string());
                return Err(err);
            }
        }
    } else {
        stream = TcpStream::connect(path).await?;
    }

    let http = "HTTP/1.1 200 Connection Established\r\n\r\n";
    client.write_all(http.as_bytes()).await?;
    client.flush().await?;

    let mut client_half = client.into_split();
    let mut remote_half = stream.into_split();
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

    let remote_to_client = tokio::spawn(async move {
        let id = format!("remote_to_client[{tunnel_id}:{}]", tokio::task::id());
        eprintln!("{id} start");
        tokio::select! {
            _ = relay(&id, &mut remote_half.0, &mut client_half.1) => {}
            _ = cancel_rx => {},
        }
        eprintln!("{id} end");
    });

    let client_to_remote = tokio::spawn(async move {
        let id = format!("client_to_remote[{}:{}]", tunnel_id, tokio::task::id());
        eprintln!("{id} start");
        let _ = relay(&id, &mut client_half.0, &mut remote_half.1).await;
        let _ = remote_half.1.shutdown().await;
        let _ = cancel_tx.send(());
        eprintln!("{id} end");
    });

    let _ = tokio::try_join!(remote_to_client, client_to_remote);

    eprintln!("tunnel[{tunnel_id}] end");

    Ok(())
}

async fn get<'a>(
    client: TcpStream,
    path: &str,
    headers: &[httparse::Header<'_>],
) -> Result<(), std::io::Error> {
    let index;
    let mut stream: TcpStream;

    if path.starts_with("http://") {
        let url = match Url::parse(path) {
            Ok(u) => u,
            Err(_) => return Err(Error::new(ErrorKind::InvalidData, "URL parse error")),
        };
        let s = format!("{}:{}", url.host().unwrap(), url.port().unwrap_or(80));
        stream = TcpStream::connect(s).await?;
        index = path.rfind(url.path());
    } else {
        let mut saddr: Option<SocketAddr> = None;
        let fd = client.as_fd();
        let s = SockRef::from(&fd);

        if let Ok(v) = s.original_dst_v4() {
            saddr = v.as_socket();
        } else if let Ok(v) = s.original_dst_v6() {
            saddr = v.as_socket();
        }

        let addr = match saddr {
            Some(v) => v,
            None => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "Unable to retrive destination IP",
                ));
            }
        };

        eprintln!("ip: {}", addr.ip().to_string());

        stream = TcpStream::connect(addr).await?;

        index = Some(0);

        /*if let Some(header) = headers.iter().find(|&&h| h.name == "Host") {
            let host = String::from_utf8_lossy(header.value);
            //eprintln!("Host: {host}");
            url = match Url::from_str(&format!("http://{host}{path}")) {
        } else {
            return Err(Error::new(ErrorKind::InvalidData, "Host is missing"));
        }*/
    }

    // Do do request

    let mut http = String::new();
    let rpath = match index {
        Some(i) => &path[i..],
        None => "/",
    };
    http += &format!("GET {} HTTP/1.1\r\n", rpath);

    for header in headers.iter() {
        http += &format!(
            "{}: {}\r\n",
            header.name,
            String::from_utf8_lossy(header.value)
        );
    }
    http += &format!("Accept: /\r\n");
    http += &format!("Connection: close\r\n");
    http += &format!("\r\n\r\n");

    stream.write_all(http.as_bytes()).await?;

    let mut buf = vec![0u8; 4096];
    let mut client_half = client.into_split();

    loop {
        let len = stream.read(&mut buf).await?;
        if len == 0 {
            break;
        }
        client_half.1.write_all(&buf[..len]).await?;
    }

    Ok(())
}

async fn not_allowed(stream: TcpStream) -> Result<(), std::io::Error> {
    let http = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
    let (_, mut writer) = stream.into_split();
    writer.write_all(http.as_bytes()).await?;
    Ok(())
}

async fn auth_request(mut stream: TcpStream) -> Result<(), std::io::Error> {
    let config = CONFIG.read().await;
    let realm = config.auth_realm.clone().unwrap_or("Tetanux".to_string());
    let http = format!(
        "HTTP/1.1 407 Proxy Authentication Required\r\n\
         Proxy-Authenticate: Basic realm=\"{realm}\"\r\n\
         Connection: close\r\n\r\n",
    );
    stream.write_all(http.as_bytes()).await?;
    Ok(())
}

async fn make_request<'a>(
    client: TcpStream,
    method: Option<&'a str>,
    path: Option<&'a str>,
    headers: Option<&[httparse::Header<'a>]>,
) -> Result<(), std::io::Error> {
    match path {
        Some(p) => match method {
            Some("CONNECT") => {
                eprintln!("CONNECT {}", p);
                tunnel(client, p).await?
            }
            Some("GET") => {
                eprintln!("GET {}", p);
                get(client, p, headers.unwrap_or(&[httparse::EMPTY_HEADER; 0])).await?
            }
            Some(m) => {
                eprint!("{} {}", &m, p);
                not_allowed(client).await?
            }
            None => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "Invalid path".to_string(),
                ));
            }
        },
        None => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Invalid path".to_string(),
            ));
        }
    }

    Ok(())
}

async fn check_auth<'a>(headers: Option<&[httparse::Header<'_>]>) -> bool {
    let config = CONFIG.read().await;
    match &config.auth {
        Some(auth) => match headers {
            Some(headers) => {
                for header in headers {
                    if header.name == "Proxy-Authorization" {
                        let value = String::from_utf8_lossy(&header.value);
                        if value.starts_with("Basic ") {
                            match base64::engine::general_purpose::STANDARD.decode(&value[6..]) {
                                Ok(v) => {
                                    let value = String::from_utf8_lossy(&v);
                                    let split: Vec<_> = value.splitn(2, ":").collect();
                                    if split.len() == 2 {
                                        return auth.user == split[0] && auth.password == split[1];
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                    }
                }
            }
            None => {}
        },
        None => return true,
    }
    false
}

async fn handle_client(client: TcpStream) -> Result<(), std::io::Error> {
    let mut buf = vec![0; 1024];
    let mut nread: usize = 0;

    loop {
        client.readable().await?;
        match client.try_read(&mut buf[nread..]) {
            Ok(n) => {
                let mut headers = [httparse::EMPTY_HEADER; 16];
                let mut req = httparse::Request::new(&mut headers);
                nread += n;

                match req.parse(&mut buf) {
                    Ok(status) => {
                        if status.is_complete() {
                            if check_auth(Some(req.headers)).await {
                                make_request(client, req.method, req.path, Some(req.headers))
                                    .await?;
                            } else {
                                auth_request(client).await?;
                            }
                            break;
                        } else {
                            // Read again
                            continue;
                        }
                    } // Parse error
                    Err(e) => {
                        return Err(Error::new(
                            ErrorKind::InvalidData,
                            format!("Parse error: {}", e.to_string()),
                        ));
                    }
                }
            } // Read error
            Err(e) => {
                if e.kind() == ErrorKind::WouldBlock {
                    continue;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    if cli.c.is_some() {
        config::load(cli.c.unwrap().as_str()).await?;
    }

    let config = CONFIG.read().await;

    match &config.pid_file {
        Some(pid_file) => {
            if let Err(e) = utils::create_pidfile(pid_file) {
                eprintln!("Unable to create pid file: {}", e.to_string());
                return Ok(());
            }
        }
        None => {}
    }

    let socket = TcpSocket::new_v4().unwrap();
    let _ = socket.set_reuseaddr(true);
    let addr =
        SocketAddr::from_str(format!("{}:{}", config.listen_addr, config.port).as_str()).unwrap();
    let _ = socket.bind(addr).unwrap();
    let listener = socket.listen(1024).unwrap();
    eprintln!("Listening on http://{}", addr);

    let config = CONFIG.read().await;

    loop {
        let (stream, addr) = listener.accept().await?;
        if config.is_allowed(&addr.ip()) {
            eprintln!("Handle client={}", addr.ip().to_string());
            tokio::spawn(async move {
                match handle_client(stream).await {
                    Ok(_) => (),
                    Err(e) => eprintln!("error: {}", e.to_string()),
                }
            });
        } else {
            eprintln!("Host {} not allowed/denied", addr.ip().to_string());
        }
    }
}
