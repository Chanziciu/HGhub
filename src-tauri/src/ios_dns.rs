//! Resolve upstream hosts independently of VPN fake-IP/blocked local DNS answers.
use once_cell::sync::Lazy;
use std::{net::{IpAddr, SocketAddr}, sync::Mutex, collections::HashMap, time::{Instant, Duration}};
use reqwest::dns::{Resolve, Resolving, Name, Addrs};

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Duration, Vec<SocketAddr>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static LOADING: Lazy<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static DNS: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder()
    .no_proxy().timeout(Duration::from_secs(8))
    .resolve_to_addrs("dns.google", &["8.8.8.8:443".parse().unwrap(), "8.8.4.4:443".parse().unwrap()])
    .build().expect("DNS client"));
static FAST_DNS: Lazy<[(reqwest::Client, &'static str); 3]> = Lazy::new(|| [
    ("dns.google", "8.8.4.4:443", "https://dns.google/resolve"),
    ("cloudflare-dns.com", "1.0.0.1:443", "https://cloudflare-dns.com/dns-query"),
    ("dns.google", "8.8.8.8:443", "https://dns.google/resolve"),
].map(|(host, address, endpoint)| {
    (reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(6))
        .connect_timeout(Duration::from_secs(3))
        .resolve(host, address.parse().unwrap()).build().expect("DNS client"), endpoint)
}));

type DnsError = Box<dyn std::error::Error + Send + Sync>;
pub fn connection_failed(host: &str) {
    if !crate::startup_perf::optimized() { return; }
    if let Some((_, _, addrs)) = CACHE.lock().unwrap().get_mut(host) {
        if addrs.len() > 1 { addrs.rotate_left(1); }
    }
}

pub fn connected(response: &reqwest::Response) {
    if !crate::startup_perf::optimized() { return; }
    if let (Some(host), Some(remote)) = (response.url().host_str(), response.remote_addr()) {
        if let Some((_, _, addrs)) = CACHE.lock().unwrap().get_mut(host) {
            if let Some(index) = addrs.iter().position(|a| a.ip() == remote.ip()) { addrs.swap(0, index); }
        }
    }
}

async fn lookup(client: &reqwest::Client, host: &str, endpoint: &str) -> Result<(Vec<SocketAddr>, Duration), DnsError> {
    let response: serde_json::Value = client.get(endpoint).header("Accept", "application/dns-json")
        .query(&[("name", host), ("type", "A")]).send().await?.error_for_status()?.json().await?;
    let answers: Vec<_> = response["Answer"].as_array().into_iter().flatten()
        .filter(|answer| answer["type"].as_u64() == Some(1)).collect();
    let addrs: Vec<SocketAddr> = answers.iter()
        .filter_map(|answer| answer["data"].as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| match ip {
            IpAddr::V4(ip) => !ip.is_unspecified() && !ip.is_loopback() && !ip.is_private()
                && !ip.is_link_local() && !ip.is_multicast() && ip.octets()[0..2] != [198, 18] && ip.octets()[0..2] != [198, 19],
            _ => false,
        }).map(|ip| SocketAddr::new(ip, 0)).collect();
    if addrs.is_empty() { return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "DNS returned no public address").into()); }
    let ttl = answers.iter().filter_map(|a| a["TTL"].as_u64()).min().unwrap_or(60).clamp(1, 3600);
    Ok((addrs, Duration::from_secs(ttl)))
}

pub struct PublicDns;
impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // Covers and media often resolve the same CDN together; issue only one DoH request.
            let gate = {
                let mut loading = LOADING.lock().unwrap();
                if loading.len() >= 256 { loading.retain(|_, gate| std::sync::Arc::strong_count(gate) > 1); }
                loading.entry(host.clone()).or_default().clone()
            };
            let _guard = if crate::startup_perf::optimized() { Some(gate.lock().await) } else { None };
            let started = Instant::now();
            let hit = {
                let mut cache = CACHE.lock().unwrap();
                cache.get_mut(&host).filter(|(time, ttl, _)| time.elapsed() < *ttl).map(|(_, _, addrs)| addrs.clone())
            };
            if let Some(addrs) = hit { return Ok(Box::new(addrs.into_iter()) as Addrs); }
            let (addrs, ttl) = if crate::startup_perf::optimized() {
                // Race separately pinned connections: a TCP-success/TLS-failure on one IP
                // does not make reqwest try the other address in a single resolver result.
                let mut tasks = tokio::task::JoinSet::new();
                for (client, endpoint) in FAST_DNS.iter() {
                    let host = host.clone();
                    tasks.spawn(async move { lookup(client, &host, endpoint).await });
                }
                let mut selected = None;
                let mut error: Option<DnsError> = None;
                while let Some(result) = tasks.join_next().await {
                    match result {
                        Ok(Ok(value)) => { selected = Some(value); tasks.abort_all(); break; },
                        Ok(Err(reason)) => error = Some(reason),
                        Err(reason) => error = Some(Box::new(reason)),
                    }
                }
                selected.ok_or_else(|| error.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "DNS unavailable").into()))?
            } else { lookup(&DNS, &host, "https://dns.google/resolve").await? };
            { let mut cache = CACHE.lock().unwrap(); if cache.len() > 256 { cache.clear(); } cache.insert(host, (Instant::now(), ttl, addrs.clone())); }
            crate::startup_perf::log(&format!("dns ms={}", started.elapsed().as_millis()));
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}
