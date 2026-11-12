//! Resolve upstream hosts independently of VPN fake-IP/blocked local DNS answers.
use once_cell::sync::Lazy;
use std::{net::{IpAddr, SocketAddr}, sync::Mutex, collections::HashMap, time::{Instant, Duration}};
use reqwest::dns::{Resolve, Resolving, Name, Addrs};

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<SocketAddr>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static LOADING: Lazy<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static DNS: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder()
    .no_proxy().timeout(Duration::from_secs(8))
    .resolve_to_addrs("dns.google", &["8.8.8.8:443".parse().unwrap(), "8.8.4.4:443".parse().unwrap()])
    .build().expect("DNS client"));

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
            let hit = CACHE.lock().unwrap().get(&host).filter(|(time,_)| time.elapsed() < Duration::from_secs(60)).map(|(_,a)| a.clone());
            if let Some(addrs) = hit { return Ok(Box::new(addrs.into_iter()) as Addrs); }
            let response: serde_json::Value = DNS.get("https://dns.google/resolve")
                .query(&[("name", host.as_str()), ("type", "A")]).send().await?.error_for_status()?.json().await?;
            let addrs: Vec<SocketAddr> = response["Answer"].as_array().into_iter().flatten()
                .filter(|answer| answer["type"].as_u64() == Some(1))
                .filter_map(|answer| answer["data"].as_str()?.parse::<IpAddr>().ok())
                .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
                .map(|ip| SocketAddr::new(ip, 0)).collect();
            if addrs.is_empty() { return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("DNS returned no address for {host}")).into()); }
            { let mut cache = CACHE.lock().unwrap(); if cache.len() > 256 { cache.clear(); } cache.insert(host, (Instant::now(), addrs.clone())); }
            crate::startup_perf::log(&format!("dns ms={}", started.elapsed().as_millis()));
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}
