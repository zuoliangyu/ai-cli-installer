use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "installer-web", about = "ai-cli-installer Web Server")]
pub struct Config {
    /// Host to bind to.
    #[arg(long, default_value = "127.0.0.1", env = "INSTALLER_HOST")]
    pub host: String,

    /// Port to listen on.
    #[arg(long, default_value_t = 3210, env = "INSTALLER_PORT")]
    pub port: u16,
}
