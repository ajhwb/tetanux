use std::net::SocketAddr;
use trust_dns_resolver::TokioAsyncResolver;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::error::ResolveError;

pub async fn resolve(config: ResolverConfig, name: &str) -> Result<Vec<SocketAddr>, ResolveError> {
    let resolver = TokioAsyncResolver::tokio(config, ResolverOpts::default());
    let res = resolver.lookup_ip(name).await?;
    let mut addrs: Vec<SocketAddr> = vec![];
    for ip in res.iter() {
        addrs.push(SocketAddr::from((ip, 443)));
    }
    Ok(addrs)
}
