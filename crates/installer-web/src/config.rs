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
    /// binding to a non-loopback host.
    #[arg(long, env = "INSTALLER_TOKEN")]
    pub token: Option<String>,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if is_loopback_host(&self.host) {
            return Ok(());
        }
        match self.token.as_deref() {
            Some(token) if token.trim().len() >= 16 => Ok(()),
            _ => Err("非本机监听必须通过 --token 或 INSTALLER_TOKEN 提供至少 16 位访问令牌".into()),
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_bind_requires_a_real_token() {
        let config = Config {
            host: "0.0.0.0".into(),
            port: 3210,
            token: None,
        };
        assert!(config.validate().is_err());
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
    }
}
