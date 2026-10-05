use clap::Parser;
use std::net::IpAddr;

#[derive(Parser, Debug, Clone)]
#[command(name = "installer-web", about = "ai-cli-installer Web Server")]
pub struct Config {
    /// Host to bind to.
    #[arg(long, default_value = "127.0.0.1", env = "INSTALLER_HOST")]
    pub host: String,

    /// Port to listen on.
    #[arg(long, default_value_t = 3210, env = "INSTALLER_PORT")]
    pub port: u16,

    /// Access token required by API and WebSocket requests. Mandatory when
    /// binding to a non-loopback host; a random one is generated otherwise.
    #[arg(long, env = "INSTALLER_TOKEN")]
    pub token: Option<String>,

    /// Extra `Host` header values accepted besides the loopback addresses
    /// (e.g. LAN IP or reverse-proxy domain). Entries without a port also
    /// match `<entry>:<port>`. Comma-separated in the env variable.
    #[arg(
        long = "allowed-host",
        env = "INSTALLER_ALLOWED_HOSTS",
        value_delimiter = ','
    )]
    pub allowed_hosts: Vec<String>,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if is_loopback_host(&self.host) {
            return Ok(());
        }
        match self.configured_token() {
            Some(token) if token.len() >= 16 => Ok(()),
            _ => Err("非本机监听必须通过 --token 或 INSTALLER_TOKEN 提供至少 16 位访问令牌".into()),
        }
    }

    pub fn configured_token(&self) -> Option<&str> {
        self.token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
    }

    /// Every `Host` header value (lower-cased, `host:port` form unless the
    /// operator configured a port-less entry) the server answers to.
    pub fn allowed_hosts(&self) -> Vec<String> {
        let port = self.port;
        let mut hosts = vec![
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ];
        let bind = self.host.trim();
        if !is_loopback_host(bind) && !is_unspecified_host(bind) {
            hosts.push(with_port(bind, port));
        }
        for entry in &self.allowed_hosts {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            hosts.push(entry.to_ascii_lowercase());
            if !has_port(entry) {
                hosts.push(with_port(entry, port));
            }
        }
        hosts.sort();
        hosts.dedup();
        hosts
    }

    /// Bound to `0.0.0.0` / `::` without any extra allowed host: remote
    /// clients will be rejected by the Host check.
    pub fn remote_bind_without_allowed_hosts(&self) -> bool {
        is_unspecified_host(self.host.trim())
            && self.allowed_hosts.iter().all(|h| h.trim().is_empty())
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || parse_ip(host).is_some_and(|ip| ip.is_loopback())
}

fn is_unspecified_host(host: &str) -> bool {
    parse_ip(host).is_some_and(|ip| ip.is_unspecified())
}

fn parse_ip(host: &str) -> Option<IpAddr> {
    host.trim_matches(['[', ']']).parse::<IpAddr>().ok()
}

fn has_port(entry: &str) -> bool {
    match entry.rsplit_once(':') {
        // Bracketed IPv6 (`[::1]:80`) or plain `host:80`; a bare IPv6 address
        // has more than one colon and no brackets, so it carries no port.
        Some((head, port)) => {
            port.parse::<u16>().is_ok() && (head.ends_with(']') || !head.contains(':'))
        }
        None => false,
    }
}

fn with_port(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{host}]:{port}"),
        _ => format!("{host}:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(host: &str, allowed: &[&str]) -> Config {
        Config {
            host: host.into(),
            port: 3210,
            token: None,
            allowed_hosts: allowed.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn remote_bind_requires_a_real_token() {
        let mut cfg = config("0.0.0.0", &[]);
        assert!(cfg.validate().is_err());
        cfg.token = Some("   short   ".into());
        assert!(cfg.validate().is_err());
        cfg.token = Some("0123456789abcdef".into());
        assert!(cfg.validate().is_ok());
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(config("localhost", &[]).validate().is_ok());
    }

    #[test]
    fn loopback_bind_allows_only_loopback_hosts() {
        let hosts = config("127.0.0.1", &[]).allowed_hosts();
        assert_eq!(hosts, ["127.0.0.1:3210", "[::1]:3210", "localhost:3210"]);
    }

    #[test]
    fn explicit_bind_and_allowed_hosts_are_added() {
        let hosts =
            config("192.168.1.5", &["Example.COM", "proxy.lan:8443", "fe80::1"]).allowed_hosts();
        assert!(hosts.contains(&"192.168.1.5:3210".to_string()));
        assert!(hosts.contains(&"example.com".to_string()));
        assert!(hosts.contains(&"example.com:3210".to_string()));
        assert!(hosts.contains(&"proxy.lan:8443".to_string()));
        assert!(!hosts.contains(&"proxy.lan:8443:3210".to_string()));
        assert!(hosts.contains(&"[fe80::1]:3210".to_string()));
    }

    #[test]
    fn unspecified_bind_is_not_a_valid_host() {
        let cfg = config("0.0.0.0", &[]);
        assert!(!cfg.allowed_hosts().iter().any(|h| h.starts_with("0.0.0.0")));
        assert!(cfg.remote_bind_without_allowed_hosts());
        assert!(!config("0.0.0.0", &["10.0.0.2"]).remote_bind_without_allowed_hosts());
    }
}
