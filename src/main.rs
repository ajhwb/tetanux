use base64::Engine;
use clap::Parser;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

mod cli;
use cli::Cli;
mod config;
use config::CONFIG;
mod resolver;
use resolver::resolve;

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

        if let Err(e) = writer.write(&buf[..nread]).await {
            eprintln!("{id}: write error: {}", e.to_string());
            break;
        } else {
            if let Err(e) = writer.flush().await {
                eprintln!("{id}: write error: {}", e.to_string());
                break;
            }
        }
    }
}

async fn tunnel(client: TcpStream, path: &str) -> Result<(), std::io::Error> {
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

    let mut client_half = client.into_split();
    let mut remote_half = stream.into_split();

    client_half.1.write_all(http.as_bytes()).await?;
    client_half.1.flush().await?;

    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(());
    let mut cancel_rx_clone = cancel_rx.clone();

    let remote_to_client = tokio::spawn(async move {
        let id = format!("remote_to_client[{tunnel_id}:{}]", tokio::task::id());
        eprintln!("{id} start");
        tokio::select! {
            _ = relay(&id, &mut remote_half.0, &mut client_half.1) => {}
            _ = cancel_rx.changed() => {},
        }
        drop(client_half.1);
        eprintln!("{id} end");
    });

    let client_to_remote = tokio::spawn(async move {
        let id = format!("client_to_remote[{}:{}]", tunnel_id, tokio::task::id());
        eprintln!("{id} start");

        tokio::select! {
            _ = relay(&id, &mut client_half.0, &mut remote_half.1) => {}
            _ = cancel_rx_clone.changed() => {}
        }
        drop(remote_half.1);
        let _ = cancel_tx.send(());
        eprintln!("{id} end");
    });

    let _ = tokio::try_join!(remote_to_client, client_to_remote);

    eprintln!("tunnel[{tunnel_id}] end");

    Ok(())
}

/*
async fn tunnel2(mut client: TcpStream, uri: &str) -> Result<(), std::io::Error> {
    eprintln!("tunnel[{}] start", tokio::task::id());
    let stream: TcpStream;
    let config = CONFIG.read().await;

    if let Some(r) = &config.doh_resolver {
        let n = uri.find(':').unwrap();
        let addrs = resolve(r.clone(), &uri[..n]).await?;
        // https://doc.rust-lang.org/std/net/trait.ToSocketAddrs.html
        stream = TcpStream::connect(&addrs[..]).await?;
    } else {
        stream = TcpStream::connect(uri).await?;
    }

    let http = "HTTP/1.1 200 Connection Established\r\n\r\n";
    client.write_all(http.as_bytes()).await?;
    client.flush().await?;

    let mut remote = stream;

    let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;

    eprintln!("tunnel[{}] end", tokio::task::id());

    Ok(())
}
    */

/*
async fn request<'a>(
    client: TcpStream,
    req: &httparse::Request<'_, '_>,
) -> Result<(), std::io::Error> {
    //let path = Path::new(req.path.unwrap());
    //let basename = path.file_name().unwrap().to_os_string();
    //let http = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
    let url = match Url::parse(req.path.unwrap()) {
        Ok(u) => u,
        Err(_) => return Err(Error::new(ErrorKind::InvalidData, "URL parse error")),
    };

    let mut http = String::new();
    http += &format!("GET {} HTTP/1.1\r\n", url.path());
    //http += &format!("Host: {}\r\n", url.host_str().unwrap());
    for header in req.headers.iter() {
        http += &format!(
            "{}: {}\r\n",
            header.name,
            String::from_utf8_lossy(header.value)
        );
    }
    http += &format!("Accept: /\r\n");
    http += &format!("Connection: close\r\n");
    http += &format!("\r\n\r\n");

    let addr = format!("{}:{}", url.host().unwrap(), url.port().unwrap_or(80));
    let mut stream = TcpStream::connect(addr).await?;
    //println!("{:#?}", http);
    stream.write(http.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = vec![0u8; 4096];
    let mut client_half = client.into_split();

    loop {
        let len = stream.read(&mut buf).await?;
        if len == 0 {
            break;
        }
        client_half.1.write(&buf[..len]).await?;
        client_half.1.flush().await?;
    }

    //let _ = writer.write(http.as_bytes()).await;
    //let _ = writer.flush().await;
    //let _ = writer.shutdown().await;

    stream.shutdown().await?;
    drop(client_half.1);

    Ok(())
}
    */

async fn get<'a>(
    client: TcpStream,
    path: &str,
    headers: &[httparse::Header<'_>],
) -> Result<(), std::io::Error> {
    let url = match Url::parse(path) {
        Ok(u) => u,
        Err(_) => return Err(Error::new(ErrorKind::InvalidData, "URL parse error")),
    };

    let mut http = String::new();
    http += &format!("GET {} HTTP/1.1\r\n", url.path());

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

    let addr = format!("{}:{}", url.host().unwrap(), url.port().unwrap_or(80));
    let mut stream = TcpStream::connect(addr).await?;

    stream.write(http.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = vec![0u8; 4096];
    let mut client_half = client.into_split();

    loop {
        let len = stream.read(&mut buf).await?;
        if len == 0 {
            break;
        }
        client_half.1.write(&buf[..len]).await?;
        client_half.1.flush().await?;
    }

    Ok(())
}

async fn not_allowed(stream: TcpStream) -> Result<(), std::io::Error> {
    let http = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
    let (_, mut writer) = stream.into_split();
    writer.write_all(http.as_bytes()).await?;
    Ok(())
}

async fn forbidden_access(stream: TcpStream) -> Result<(), std::io::Error> {
    let http = "HTTP/1.1 403 Forbidden\r\n\r\n";
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
                get(client, p, headers.unwrap()).await?
            }
            Some(_) => {
                eprint!("{} {}", method.unwrap(), p);
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

async fn handle_client(client: TcpStream, addr: SocketAddr) -> Result<(), std::io::Error> {
    let mut buf = vec![0; 1024];
    let mut nread: usize = 0;
    let config = CONFIG.read().await;

    if !config.is_allowed(&addr.ip()) {
        return forbidden_access(client).await;
    }

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

/*
async fn handle_client2(client: TcpStream, addr: SocketAddr) -> Result<(), std::io::Error> {
    let mut buf = vec![0; 1024];
    let mut nread: usize = 0;
    let config = CONFIG.read().await;

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
                            if req.method.unwrap() == "CONNECT" {
                                eprintln!("CONNECT {}", req.path.unwrap());
                                if config.is_allowed(&addr.ip()) {
                                    tunnel(client, req.path.unwrap()).await?;
                                } else {
                                    forbidden_access(client).await?;
                                }
                            } else if req.method.unwrap() == "GET" {
                                eprintln!("GET {}", req.path.unwrap());
                                let mut headers: HashMap<&str, String> = HashMap::new();
                                let iter = req.headers.iter();
                                for h in iter {
                                    headers.insert(
                                        h.name,
                                        String::from_utf8(h.value.to_vec()).unwrap(),
                                    );
                                }
                                request(client, &req).await?;
                            } else {
                                not_allowed(client).await?;
                            }
                            break;
                        } else {
                            continue;
                        }
                    } // Parse error
                    Err(e) => {
                        return Err(Error::new(ErrorKind::InvalidData, e.to_string()));
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
*/

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    if cli.c.is_some() {
        config::load(cli.c.unwrap().as_str()).await?;
    }

    let config = CONFIG.read().await;
    let addr = SocketAddr::from_str(format!("{}:{}", config.listen_addr, config.port).as_str())?;
    let listener = TcpListener::bind(addr).await?;
    eprintln!("Listening on http://{}", addr);

    loop {
        let (stream, addr) = listener.accept().await?;
        eprintln!("Handle client={}", addr.ip().to_string());
        tokio::spawn(async move {
            match handle_client(stream, addr).await {
                Ok(_) => (),
                Err(e) => eprintln!("error: {}", e.to_string()),
            }
        });
    }
}
