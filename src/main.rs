use bytes::Bytes;
use std::convert::Infallible;
use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use http_body_util::Full;
use hyper::{Method, Request, Response, StatusCode};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};

async fn relay(id: &str, reader: &mut OwnedReadHalf, writer: &mut OwnedWriteHalf) -> () {
    let mut buf = vec![0; 10 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(n) => {
                if n == 0 {
                    break;
                } else {
                    println!("{}: read result={}", id, n);
                    match writer.write(&buf[..n]).await {
                        Ok(n) => {
                            let _ = writer.flush().await;
                            println!("{}: write result={}", id, n);
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e.to_string());
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
                eprintln!("Error: {}", e.to_string());
                continue;
            }
        }
    }
}

async fn tunnel(client: TcpStream, uri: &str) -> Result<(), std::io::Error> {
    let stream = TcpStream::connect(uri).await?;
    //let remote = Arc::new(RwLock::new(stream));
    println!("connected to {}", uri);
    let http = "HTTP/1.1 200 Connection Established\r\n\r\n";
    //let http = b"HTTP/1.1 405 Method Not Allowed\r\n\r\n";

    let mut client_half = client.into_split();
    let mut remote_half = stream.into_split();

    client_half.1.write_all(http.as_bytes()).await?;
    client_half.1.flush().await?;

    let remote_to_client = tokio::spawn(async move {
        let id = format!(
            "remote_to_client #{}/{:?}",
            tokio::task::id(),
            std::thread::current().id()
        );
        println!("{id} task start");
        relay(&id, &mut remote_half.0, &mut client_half.1).await;
        drop(client_half.1);
        println!("{id} task end");
    });

    let client_to_remote = tokio::spawn(async move {
        let id = format!(
            "client_to_remote #{}/{:?}",
            tokio::task::id(),
            std::thread::current().id()
        );
        println!("{id} task start");
        relay(&id, &mut client_half.0, &mut remote_half.1).await;
        drop(remote_half.1);
        println!("{id} task end");
    });

    let _ = remote_to_client.await;
    let _ = client_to_remote.await;

    Ok(())
}

async fn request(stream: TcpStream, uri: &str) -> Result<(), std::io::Error> {
    let http = "HTTP/1.1 405 Method Not Allowed\r\n\r\n";
    let (_, mut writer) = stream.into_split();
    let _ = writer.write(http.as_bytes()).await;
    let _ = writer.flush().await;
    let _ = writer.shutdown().await;
    Ok(())
}

async fn handle_client(client: TcpStream, addr: SocketAddr) -> Result<(), std::io::Error> {
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
                            //println!("{:#?}", req);
                            if req.method.unwrap() == "CONNECT" {
                                println!("tunnel to: {}", req.path.unwrap());
                                tunnel(client, req.path.unwrap()).await?;
                            } else if req.method.unwrap() == "GET" {
                                println!("GET: {}", req.path.unwrap());
                                request(client, req.path.unwrap()).await?;
                            }
                            break;
                        } else {
                            continue;
                        }
                    }
                    Err(e) => {
                        eprintln!("HTTP parse error: {e}");
                        break;
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
    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();

    let listener = TcpListener::bind(addr).await?;
    eprintln!("Listening on http://{}", addr);
    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            match handle_client(stream, addr).await {
                Ok(_) => (),
                Err(_) => eprintln!("error"),
            }
        });
    }
}
