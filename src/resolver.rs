use hickory_resolver::ResolveError;
use hickory_resolver::Resolver;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::name_server::TokioConnectionProvider;
use std::net::SocketAddr;

// https://doc.rust-lang.org/std/net/trait.ToSocketAddrs.html
pub async fn resolve(
    config: ResolverConfig,
    name: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, ResolveError> {
    let resolver =
        Resolver::builder_with_config(config, TokioConnectionProvider::default()).build();

    let addrs = resolver
        .lookup_ip(name)
        .await?
        .iter()
        .map(|ip| SocketAddr::from((ip, port)))
        .collect();

    Ok(addrs)
}
