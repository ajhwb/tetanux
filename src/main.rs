use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

mod cli;
use cli::Cli;
mod config;
use config::CONFIG;

async fn relay(id: &str, reader: &mut OwnedReadHalf, writer: &mut OwnedWriteHalf) -> () {
    let mut buf = vec![0; 10 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(n) => {
                if n == 0 {
                    break;
                } else {
                    // println!("{}: read result={}", id, n);
                    match writer.write(&buf[..n]).await {
                        Ok(n) => {
                            let _ = writer.flush().await;
                            // println!("{}: write result={}", id, n);
                        }
                        Err(e) => {
                            eprintln!("{id}: error: {}", e.to_string());
                            if e.kind() == ErrorKind::ConnectionReset {
                                break;
                            } else {
                                continue;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("{id}: error: {}", e.to_string());
                continue;
            }
        }
    }
}

async fn tunnel(client: TcpStream, uri: &str) -> Result<(), std::io::Error> {
    let stream = TcpStream::connect(uri).await?;
    let http = "HTTP/1.1 200 Connection Established\r\n\r\n";

    let mut client_half = client.into_split();
    let mut remote_half = stream.into_split();

    client_half.1.write_all(http.as_bytes()).await?;
    client_half.1.flush().await?;

    let remote_to_client = tokio::spawn(async move {
        let id = format!("remote_to_client[{}]", tokio::task::id());
        println!("{id} task start");
        relay(&id, &mut remote_half.0, &mut client_half.1).await;
        drop(client_half.1);
        println!("{id} task end");
    });

    let client_to_remote = tokio::spawn(async move {
        let id = format!("client_to_remote[{}]", tokio::task::id());
        println!("{id}: task start");
        relay(&id, &mut client_half.0, &mut remote_half.1).await;
        drop(remote_half.1);
        println!("{id}: task end");
    });

    let _ = remote_to_client.await;
    let _ = client_to_remote.await;

    Ok(())
}

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

async fn not_allowed(stream: TcpStream) -> Result<(), std::io::Error> {
    let http = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
    let (_, mut writer) = stream.into_split();
    writer.write(http.as_bytes()).await?;
    writer.flush().await?;
    writer.shutdown().await?;
    Ok(())
}

async fn handle_client(client: TcpStream, _addr: SocketAddr) -> Result<(), std::io::Error> {
    let mut buf = vec![0; 1024];
    let mut nread: usize = 0;

    loop {
        client.readable().await?;
        match client.try_read(&mut buf[nread..]) {
            Ok(n) => {
                let mut headers = [httparse::EMPTY_HEADER; 64];
                let mut req = httparse::Request::new(&mut headers);
                nread += n;

                match req.parse(&mut buf) {
                    Ok(status) => {
                        if status.is_complete() {
                            if req.method.unwrap() == "CONNECT" {
                                println!("CONNECT {}", req.path.unwrap());
                                tunnel(client, req.path.unwrap()).await?;
                            } else if req.method.unwrap() == "GET" {
                                println!("GET {}", req.path.unwrap());
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
                    }
                    Err(e) => {
                        return Err(Error::new(ErrorKind::InvalidData, e.to_string()));
                    }
                }
            }
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
        config::load(cli.c.unwrap().as_str())?;
    }
    let config = CONFIG.read().unwrap();

    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();
    SocketAddr::from_str(format!("{}:{}", config.listen_addr, config.port).as_str())?;

    let listener = TcpListener::bind(addr).await?;
    eprintln!("Listening on http://{}", addr);
    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            match handle_client(stream, addr).await {
                Ok(_) => (),
                Err(e) => eprintln!("error: {}", e.to_string()),
            }
        });
    }
}
