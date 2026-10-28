#![windows_subsystem = "windows"]

use clap::{Parser, Subcommand};
use lianli_controller::bridge::{attach_virtual, detach_all_virtual, get_diagnostics, list_ports, VirtualDongleServer};
use lianli_controller::gui::run_gui;
use lianli_controller::hardware::{autostart, c3_link, is_service_running};

#[derive(Parser)]
#[command(name = "lianli-controller", version)]
#[command(about = "Virtual Lian Li wireless dongle for L-Connect 3, backed by an ESP32-C3 RF Master")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Start the GUI hidden in the tray (used by the logon task)
    #[arg(long)]
    tray: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the GUI (default)
    Gui,

    /// Find the ESP32-C3 and check for other controllers
    Detect,

    /// Show USB/IP driver, virtual ports, L-Connect services and autostart state
    Status,

    /// Virtual dongle bridge
    Bridge {
        #[command(subcommand)]
        sub: BridgeCommands,
    },

    /// USB/IP virtual ports
    Ports {
        #[command(subcommand)]
        sub: PortCommands,
    },

    /// Start with Windows (logon task, highest privileges)
    Autostart {
        #[command(subcommand)]
        sub: AutostartCommands,
    },
}

#[derive(Subcommand)]
enum BridgeCommands {
    /// Run the bridge in the foreground (blocks)
    Start {
        #[arg(short, long, default_value = "127.0.0.1:3240")]
        bind: String,
        /// Serial port of the ESP32-C3 (e.g. COM3); auto-detected when omitted
        #[arg(long)]
        c3: Option<String>,
    },
}

#[derive(Subcommand)]
enum PortCommands {
    /// List attached USB/IP devices
    List,
    /// Attach the virtual TX and RX dongles
    Attach,
    /// Detach all virtual dongle ports
    Detach,
}

#[derive(Subcommand)]
enum AutostartCommands {
    On,
    Off,
}

/// GUI-subsystem exe: attach to the parent console so CLI output is visible.
fn attach_console() {
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows::core::w;
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        let cur = GetStdHandle(STD_OUTPUT_HANDLE).unwrap_or_default();
        if (cur.is_invalid() || cur.0.is_null())
            && let Ok(h) = CreateFileW(w!("CONOUT$"), 0xC0000000, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None) {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, h);
                let _ = SetStdHandle(STD_ERROR_HANDLE, h);
            }
    }
}

/// One instance at a time: two bridges would fight over the serial port, TCP 3240 and the virtual dongle.
/// A second launch brings the running window to the front and exits.
fn already_running() -> bool {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    use windows::core::w;
    unsafe {
        // handle intentionally leaked: the mutex must live as long as the process
        let _ = CreateMutexW(None, false, w!("Global\\LianLiWirelessController"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(h) = FindWindowW(w!("LianLiControllerMainClass"), None) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        true
    }
}

fn main() {
    // Logs and config live next to the exe, also when started elevated or by the logon task
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
        let _ = std::env::set_current_dir(dir);
    }

    let args: Vec<String> = std::env::args().collect();
    let runs_bridge = args.len() <= 1 || args[1..].iter().all(|a| a == "gui" || a == "--tray") || args.get(1).is_some_and(|a| a == "bridge");
    if runs_bridge {
        if already_running() {
            return;
        }
        autostart::follow_exe();
    }
    if args.len() <= 1 || args[1..].iter().all(|a| a == "gui" || a == "--tray") {
        run_gui(args.iter().any(|a| a == "--tray"));
        return;
    }

    attach_console();
    let cli = Cli::parse();
    match cli.command {
        None | Some(Commands::Gui) => run_gui(cli.tray),
        Some(Commands::Detect) => match c3_link::detect() {
            Ok(port) => println!("ESP32-C3 RF Master on {}", port),
            Err((msg, _)) => eprintln!("{}", msg),
        },
        Some(Commands::Status) => {
            let diag = get_diagnostics();
            println!("usbip.exe:      {}", diag.usbip_exe.map(|p| p.display().to_string()).unwrap_or_else(|| "missing (winget install vadimgrn.usbip-win2)".into()));
            println!("USB/IP driver:  {}", if diag.driver_installed { "installed" } else { "missing" });
            match list_ports() {
                Ok(ports) => {
                    println!("Virtual ports:  {}", ports.len());
                    for p in ports {
                        println!("  port {:02}: {:04X}:{:04X} -> {}", p.port, p.vendor_id, p.product_id, p.uri);
                    }
                }
                Err(e) => println!("Virtual ports:  error: {}", e),
            }
            for s in ["LConnectService", "LConnectServiceWatcher"] {
                println!("{:<22} {}", s, if is_service_running(s) { "running" } else { "stopped" });
            }
            println!("Autostart:      {}", if autostart::is_enabled() { "on" } else { "off" });
        }
        Some(Commands::Bridge { sub: BridgeCommands::Start { bind, c3 } }) => {
            let mut server = VirtualDongleServer::new();
            let _ = detach_all_virtual();
            let port = match c3.map(Ok).unwrap_or_else(c3_link::detect) {
                Ok(p) => p,
                Err((msg, _)) => {
                    eprintln!("{}", msg);
                    return;
                }
            };
            println!("RF Master: ESP32-C3 on {}", port);
            let link = c3_link::C3Link::start(&port, server.air_packet_sink());
            server.set_forwarder(move |pkt| link.forward(pkt));
            if let Err(e) = server.start(&bind) {
                eprintln!("Failed to bind {}: {}", bind, e);
                return;
            }
            match attach_virtual() {
                Ok(()) => println!("Virtual dongle attached (TX 0x8040, RX 0x8041). Ctrl+C to stop."),
                Err(e) => eprintln!("Attach failed: {}", e),
            }
            while server.is_running() {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        Some(Commands::Ports { sub }) => match sub {
            PortCommands::List => match list_ports() {
                Ok(ports) if ports.is_empty() => println!("No USB/IP ports attached."),
                Ok(ports) => {
                    for p in ports {
                        println!("port {:02}: {} [{:04X}:{:04X}] -> {}", p.port, p.description, p.vendor_id, p.product_id, p.uri);
                    }
                }
                Err(e) => eprintln!("Error: {}", e),
            },
            PortCommands::Attach => match attach_virtual() {
                Ok(()) => println!("Attached TX and RX."),
                Err(e) => eprintln!("Attach failed: {}", e),
            },
            PortCommands::Detach => match detach_all_virtual() {
                Ok(n) => println!("Detached {} port(s).", n),
                Err(e) => eprintln!("Detach failed: {}", e),
            },
        },
        Some(Commands::Autostart { sub }) => match autostart::set_enabled(matches!(sub, AutostartCommands::On)) {
            Ok(()) => println!("Autostart {}", if autostart::is_enabled() { "on" } else { "off" }),
            Err(e) => eprintln!("Error: {}", e),
        },
    }
}
