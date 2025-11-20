use hickory_resolver::ResolveError;
use hickory_resolver::Resolver;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::name_server::TokioConnectionProvider;
use std::net::SocketAddr;

pub async fn resolve(config: ResolverConfig, name: &str) -> Result<Vec<SocketAddr>, ResolveError> {
    let resolver =
        Resolver::builder_with_config(config, TokioConnectionProvider::default()).build();
    let mut addrs: Vec<SocketAddr> = vec![];
    for ip in resolver.lookup_ip(name).await?.iter() {
        addrs.push(SocketAddr::from((ip, 443)));
    }
    Ok(addrs)
}
