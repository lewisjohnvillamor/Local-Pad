//! The localpad command line interface. `localpad serve` is the product;
//! everything else manages or inspects a running server.

use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use localpad_server::config::{ServerConfig, DEFAULT_ADMIN_PORT, DEFAULT_CONTROLLER_PORT};

#[derive(Parser)]
#[command(
    name = "localpad",
    version,
    about = "Use your phone as a trackpad, keyboard, gamepad or motion controller."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the server and open the admin dashboard.
    Serve(ServeArgs),
    /// Show the status of a running server.
    Status(AdminArgs),
    /// Create a fresh pairing code on a running server.
    Pair(AdminArgs),
    /// List paired devices on a running server.
    Devices(AdminArgs),
    /// List available controller profiles.
    Profiles(AdminArgs),
    /// Inspect or import profiles.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Manage the local certificate authority.
    Certificate {
        #[command(subcommand)]
        command: CertificateCommand,
    },
    /// Check this machine's readiness to run LocalPad.
    Doctor,
    /// Stop a running server.
    Stop(AdminArgs),
    /// Install LocalPad as a background service (not yet available).
    Install,
    /// Remove the background service (not yet available).
    Uninstall,
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Initial controller profile.
    #[arg(long, default_value = "touchpad")]
    profile: String,
    /// Loopback admin port.
    #[arg(long, default_value_t = DEFAULT_ADMIN_PORT)]
    admin_port: u16,
    /// LAN HTTPS controller port.
    #[arg(long, default_value_t = DEFAULT_CONTROLLER_PORT)]
    controller_port: u16,
    /// Do not open the desktop browser.
    #[arg(long)]
    no_open: bool,
    /// Development only: serve the controller over plain HTTP. Motion
    /// controls will be unavailable on phones.
    #[arg(long)]
    insecure_http: bool,
    /// Approve each connection in the admin dashboard.
    #[arg(long)]
    require_approval: bool,
    /// Override the LAN bind address.
    #[arg(long)]
    bind: Option<IpAddr>,
    /// Accept non-private bind addresses. Dangerous.
    #[arg(long)]
    allow_remote: bool,
    /// Show input in the dashboard without driving the mouse or keyboard.
    #[arg(long)]
    no_native_output: bool,
    /// Log level: error, warn, info, debug or trace.
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[derive(clap::Args)]
struct AdminArgs {
    /// Admin port of the running server.
    #[arg(long, default_value_t = DEFAULT_ADMIN_PORT)]
    admin_port: u16,
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Print one profile as JSON.
    Show {
        name: String,
        #[arg(long, default_value_t = DEFAULT_ADMIN_PORT)]
        admin_port: u16,
    },
    /// Validate and install a layout JSON file.
    Import { file: PathBuf },
}

#[derive(Subcommand)]
enum CertificateCommand {
    /// Print instructions for trusting the LocalPad CA on this computer.
    Install,
    /// Write the public CA certificate to the current directory.
    Export,
}

fn admin_base(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn admin_get(port: u16, path: &str) -> anyhow::Result<serde_json::Value> {
    let response = ureq::get(&format!("{}{path}", admin_base(port)))
        .call()
        .with_context(|| {
            format!(
                "could not reach a LocalPad server on port {port}; \
                 is `localpad serve` running?"
            )
        })?;
    Ok(response.into_json()?)
}

fn admin_post(port: u16, path: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let response = ureq::post(&format!("{}{path}", admin_base(port)))
        .set("X-LocalPad-Admin", "1")
        .send_json(body)
        .with_context(|| {
            format!(
                "could not reach a LocalPad server on port {port}; \
                 is `localpad serve` running?"
            )
        })?;
    Ok(response.into_json()?)
}

fn data_dir() -> anyhow::Result<PathBuf> {
    ServerConfig::default().resolve_data_dir()
}

fn print_terminal_qr(url: &str) {
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let rendered = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{rendered}");
        }
        Err(_) => println!("(could not render a QR code; open {url} manually)"),
    }
}

fn serve(args: ServeArgs) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&args.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dir = data_dir()?;
    let layouts_dir = dir.join("layouts");
    let _ = std::fs::create_dir_all(&layouts_dir);

    let mut config = ServerConfig {
        admin_port: args.admin_port,
        controller_port: args.controller_port,
        insecure_http: args.insecure_http,
        require_approval: args.require_approval,
        allow_remote: args.allow_remote,
        profile: args.profile,
        layouts_dir: Some(layouts_dir),
        no_native_output: args.no_native_output,
        ..Default::default()
    };
    if let Some(bind) = args.bind {
        config.bind_addr = bind;
    }

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let server = localpad_server::start(config).await?;
        let admin_url = format!("http://127.0.0.1:{}/admin", server.admin_addr.port());

        println!("LocalPad {}", env!("CARGO_PKG_VERSION"));
        println!("Admin:       {admin_url}");
        println!("Controller:  {}", server.controller_url);
        println!(
            "Pairing:     {} (expires in {} minutes)",
            server.pairing.code,
            server.pairing.expires_in.as_secs().div_ceil(60)
        );
        println!("Network:     {}", server.state.network.lan_ip);
        println!();
        print_terminal_qr(&server.pairing.url);
        println!();
        println!("Waiting for a controller... press Ctrl-C to stop.");

        if !args.no_open {
            if let Err(e) = open::that_detached(&admin_url) {
                tracing::warn!(error = %e, "could not open the browser; open {admin_url} yourself");
            }
        }

        let state = server.state.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                println!("\nStopping; releasing all inputs.");
                state.shutdown();
            }
        });

        server.wait().await;
        Ok::<(), anyhow::Error>(())
    })
}

fn doctor() -> anyhow::Result<()> {
    let mut problems = 0u32;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("{} {name}: {detail}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            problems += 1;
        }
    };

    check(
        "platform",
        true,
        format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
    );

    match local_ip_address_check() {
        Ok(ip) => check("lan address", true, ip),
        Err(e) => check("lan address", false, e),
    }

    for port in [DEFAULT_ADMIN_PORT, DEFAULT_CONTROLLER_PORT] {
        let free = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
        check(
            "port",
            free,
            if free {
                format!("{port} is available")
            } else {
                format!("{port} is in use (a LocalPad server may already be running)")
            },
        );
    }

    let dir = data_dir()?;
    let ca = dir.join("certs").join("localpad-ca.crt");
    check(
        "certificate authority",
        true,
        if ca.exists() {
            format!("present at {}", ca.display())
        } else {
            "will be created on first `localpad serve`".to_string()
        },
    );

    match std::env::consts::OS {
        "linux" => {
            let ok = localpad_output_linux::uinput_available();
            check(
                "uinput",
                ok,
                if ok {
                    "/dev/uinput is writable".to_string()
                } else {
                    "cannot write /dev/uinput. Add a udev rule:\n     \
                     echo 'KERNEL==\"uinput\", MODE=\"0660\", GROUP=\"input\", OPTIONS+=\"static_node=uinput\"' \
                     | sudo tee /etc/udev/rules.d/60-localpad.rules\n     \
                     sudo udevadm trigger && sudo usermod -aG input $USER (then log out and in)"
                        .to_string()
                },
            );
        }
        "macos" => {
            check(
                "accessibility",
                true,
                "macOS asks for the Accessibility permission on first input; \
                 grant it to the app or terminal that runs `localpad serve` in \
                 System Settings, Privacy & Security, Accessibility"
                    .to_string(),
            );
        }
        "windows" => {
            check(
                "virtual controller",
                true,
                "keyboard and mouse need no driver; the virtual Xbox \
                 controller (ViGEmBus) ships in a later release"
                    .to_string(),
            );
        }
        _ => {}
    }

    if problems == 0 {
        println!("\nEverything looks ready. Run: localpad serve");
    } else {
        println!("\n{problems} problem(s) found.");
        std::process::exit(1);
    }
    Ok(())
}

fn local_ip_address_check() -> Result<String, String> {
    // Reuse the server's discovery through a throwaway call.
    match std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| {
        s.connect("192.168.255.255:80")?;
        s.local_addr()
    }) {
        Ok(addr) if !addr.ip().is_loopback() => Ok(addr.ip().to_string()),
        _ => Err("could not determine a LAN address; check the network connection".to_string()),
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve(args),
        Command::Status(args) => {
            let status = admin_get(args.admin_port, "/api/status")?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::Pair(args) => {
            let pairing = admin_post(args.admin_port, "/api/pairing/new", serde_json::json!({}))?;
            let code = pairing["code"].as_str().unwrap_or("?");
            let url = pairing["url"].as_str().unwrap_or("?");
            println!("Pairing code: {code} (expires in 5 minutes)");
            print_terminal_qr(url);
            Ok(())
        }
        Command::Devices(args) => {
            let status = admin_get(args.admin_port, "/api/status")?;
            let devices = status["devices"].as_array().cloned().unwrap_or_default();
            if devices.is_empty() {
                println!("No devices have paired since the server started.");
            }
            for device in devices {
                println!(
                    "{}  {}  {}",
                    device["deviceId"].as_str().unwrap_or("?"),
                    device["name"].as_str().unwrap_or("?"),
                    if device["connected"].as_bool().unwrap_or(false) {
                        "connected"
                    } else {
                        "paired"
                    }
                );
            }
            Ok(())
        }
        Command::Profiles(args) => {
            let layouts = admin_get(args.admin_port, "/api/layouts")?;
            for layout in layouts["layouts"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{:<18} {}",
                    layout["id"].as_str().unwrap_or("?"),
                    layout["name"].as_str().unwrap_or("?")
                );
            }
            Ok(())
        }
        Command::Profile { command } => match command {
            ProfileCommand::Show { name, admin_port } => {
                let layouts = admin_get(admin_port, "/api/layouts")?;
                let found = layouts["layouts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|l| l["id"].as_str() == Some(name.as_str()));
                match found {
                    Some(layout) => {
                        println!("{}", serde_json::to_string_pretty(&layout)?);
                        Ok(())
                    }
                    None => anyhow::bail!("no profile named {name:?}"),
                }
            }
            ProfileCommand::Import { file } => {
                let bytes = std::fs::read(&file)
                    .with_context(|| format!("could not read {}", file.display()))?;
                let layout = localpad_core::layout::Layout::parse(&bytes)
                    .map_err(|e| anyhow::anyhow!("invalid layout: {e}"))?;
                let dir = data_dir()?.join("layouts");
                std::fs::create_dir_all(&dir)?;
                let dest = dir.join(format!("{}.json", layout.id));
                std::fs::write(&dest, &bytes)?;
                println!(
                    "Imported {:?} to {}. Restart `localpad serve` to use it.",
                    layout.name,
                    dest.display()
                );
                Ok(())
            }
        },
        Command::Certificate { command } => match command {
            CertificateCommand::Export => {
                let ca = data_dir()?.join("certs").join("localpad-ca.crt");
                if !ca.exists() {
                    anyhow::bail!(
                        "no certificate authority yet; run `localpad serve` once first"
                    );
                }
                let dest = std::env::current_dir()?.join("localpad-ca.crt");
                std::fs::copy(&ca, &dest)?;
                println!("Wrote {}", dest.display());
                println!("This file contains only the public certificate; the key never leaves this machine.");
                Ok(())
            }
            CertificateCommand::Install => {
                let ca = data_dir()?.join("certs").join("localpad-ca.crt");
                println!("The phone needs the LocalPad CA to use motion controls over HTTPS.");
                println!("Easiest path: open the /setup page shown in the admin dashboard on the phone.");
                println!();
                println!("To trust it on this computer as well:");
                match std::env::consts::OS {
                    "macos" => println!(
                        "  security add-trusted-cert -k ~/Library/Keychains/login.keychain-db {}",
                        ca.display()
                    ),
                    "linux" => println!(
                        "  sudo cp {} /usr/local/share/ca-certificates/localpad.crt && sudo update-ca-certificates",
                        ca.display()
                    ),
                    "windows" => println!(
                        "  certutil -user -addstore Root {}",
                        ca.display()
                    ),
                    _ => {}
                }
                Ok(())
            }
        },
        Command::Doctor => doctor(),
        Command::Stop(args) => {
            admin_post(args.admin_port, "/api/shutdown", serde_json::json!({}))?;
            println!("Asked the server to stop.");
            Ok(())
        }
        Command::Install | Command::Uninstall => {
            anyhow::bail!(
                "background installation ships in a later release; \
                 for now run `localpad serve` directly"
            );
        }
    }
}
