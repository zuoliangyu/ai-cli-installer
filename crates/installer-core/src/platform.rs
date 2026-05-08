use crate::error::{AppError, Result};

pub fn current() -> Result<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("win32-x64");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Ok("win32-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Ok("darwin-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok("darwin-arm64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok(if linux::is_musl() { "linux-x64-musl" } else { "linux-x64" });
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Ok(if linux::is_musl() { "linux-arm64-musl" } else { "linux-arm64" });
    }
    #[allow(unreachable_code)]
    Err(AppError::UnsupportedPlatform(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    pub fn is_musl() -> bool {
        Path::new("/lib/libc.musl-x86_64.so.1").exists()
            || Path::new("/lib/libc.musl-aarch64.so.1").exists()
    }
}
