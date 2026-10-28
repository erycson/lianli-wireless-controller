use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, UpdateWindow, DEFAULT_GUI_FONT, HBRUSH, WHITE_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::bridge::rf_protocol::AIR_NODE_TTL;
use crate::bridge::{attach_virtual, detach_all_virtual, list_ports, VirtualDongleServer};

const WM_TRAY_ICON: u32 = WM_APP + 1;
const ID_TIMER_REFRESH: usize = 1;

const IDM_TRAY_OPEN: usize = 401;
const IDM_TRAY_AUTOSTART: usize = 402;
const ID_CHK_LOG: usize = 501;
const IDM_TRAY_EXIT: usize = 404;

pub fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// What the startup thread found: the GUI only displays it.
#[derive(Clone)]
enum Radio {
    Searching(String),
    Conflict(String),
    Running(String),
}

struct AppState {
    server: Arc<Mutex<VirtualDongleServer>>,
    radio: Arc<Mutex<Radio>>,
    lbl_radio: HWND,
    lbl_bridge: HWND,
    list_fans: HWND,
    edit_log: HWND,
    log_seq: u64,
}

unsafe impl Send for AppState {}

static APP_STATE: Mutex<Option<AppState>> = Mutex::new(None);

unsafe fn send_msg(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

unsafe fn set_text(hwnd: HWND, s: &str) {
    let w = to_wide(s);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

unsafe fn create_control(class: PCWSTR, text: PCWSTR, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, r: (i32, i32, i32, i32), parent: HWND) -> HWND {
    unsafe {
        let inst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let h = CreateWindowExW(ex, class, text, WS_CHILD | WS_VISIBLE | style, r.0, r.1, r.2, r.3, Some(parent), None, Some(inst), None)
            .unwrap_or_default();
        let _ = send_msg(h, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT).0 as usize, 1);
        h
    }
}

/// Icon embedded by build.rs (assets/icon.ico, resource id 1).
unsafe fn app_icon() -> HICON {
    unsafe {
        let inst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        LoadIconW(Some(inst), PCWSTR(1 as _)).or_else(|_| LoadIconW(None, IDI_APPLICATION)).unwrap_or_default()
    }
}

/// `hidden`: start in the tray only (used by the logon task).
pub fn run_gui(hidden: bool) {
    unsafe {
        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icex);

        let h_instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let class_name = w!("LianLiControllerMainClass");
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: h_instance,
            hIcon: app_icon(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            lpszClassName: class_name,
            hIconSm: app_icon(),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wnd_class);

        let (win_w, win_h) = (760, 600);
        let x = (GetSystemMetrics(SM_CXSCREEN) - win_w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - win_h) / 2;
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            PCWSTR(to_wide(concat!("Lian Li Wireless Controller ", env!("CARGO_PKG_VERSION"))).as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            x, y, win_w, win_h,
            None, None, Some(h_instance), None,
        ) else {
            return;
        };

        if !hidden {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                init_controls(hwnd);
                init_tray(hwnd);
                let _ = SetTimer(Some(hwnd), ID_TIMER_REFRESH, 1000, None);
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == ID_TIMER_REFRESH {
                    on_timer_tick();
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                if wparam.0 & 0xFFFF == ID_CHK_LOG {
                    let on = send_msg(HWND(lparam.0 as _), BM_GETCHECK, 0, 0).0 == BST_CHECKED.0 as isize;
                    crate::traffic_log::set_file_logging(on);
                }
                LRESULT(0)
            }
            WM_TRAY_ICON => {
                let event = (lparam.0 & 0xFFFF) as u32;
                if event == WM_LBUTTONDBLCLK || event == WM_LBUTTONUP {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                } else if event == WM_RBUTTONUP {
                    show_tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = detach_all_virtual();
                remove_tray(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn init_controls(hwnd: HWND) {
    unsafe {
        let none = WINDOW_EX_STYLE::default();
        let _ = create_control(w!("BUTTON"), w!(" Controller "), WINDOW_STYLE(BS_GROUPBOX as u32), none, (15, 8, 715, 70), hwnd);
        let lbl_radio = create_control(w!("STATIC"), w!("Looking for the ESP32-C3..."), WINDOW_STYLE(0), none, (30, 28, 690, 18), hwnd);
        let lbl_bridge = create_control(w!("STATIC"), w!(""), WINDOW_STYLE(0), none, (30, 50, 690, 18), hwnd);

        let _ = create_control(w!("BUTTON"), w!(" Fans heard over the air "), WINDOW_STYLE(BS_GROUPBOX as u32), none, (15, 84, 715, 200), hwnd);
        let list_fans = create_control(
            w!("SysListView32"), PCWSTR::null(),
            WINDOW_STYLE(LVS_REPORT | LVS_NOSORTHEADER), WS_EX_CLIENTEDGE, (30, 104, 685, 168), hwnd,
        );
        let _ = send_msg(list_fans, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, (LVS_EX_FULLROWSELECT | LVS_EX_GRIDLINES) as isize);
        for (i, (name, width)) in [("Group (MAC)", 120), ("Controller", 120), ("rx", 40), ("Fans", 40), ("RPM", 170), ("PWM", 120), ("Last seen", 70)].iter().enumerate() {
            let mut text = to_wide(name);
            let col = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_mut_ptr()), ..Default::default() };
            let _ = send_msg(list_fans, LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
        }

        let _ = create_control(w!("BUTTON"), w!(" Live log "), WINDOW_STYLE(BS_GROUPBOX as u32), none, (15, 290, 715, 265), hwnd);
        let edit_log = create_control(
            w!("EDIT"), w!(""),
            WINDOW_STYLE(ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32) | WS_VSCROLL | WS_HSCROLL,
            WS_EX_CLIENTEDGE, (30, 334, 685, 208), hwnd,
        );
        let chk_log = create_control(
            w!("BUTTON"), w!("Save to proxy_traffic.log (10 MB, keeps last 3)"),
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32), none, (30, 310, 400, 20), hwnd,
        );
        let _ = SetWindowLongPtrW(chk_log, GWLP_ID, ID_CHK_LOG as isize);
        let _ = send_msg(chk_log, BM_SETCHECK, crate::traffic_log::file_logging() as usize, 0);

        let server = Arc::new(Mutex::new(VirtualDongleServer::new()));
        let radio = Arc::new(Mutex::new(Radio::Searching("Looking for the ESP32-C3...".into())));
        spawn_startup(Arc::clone(&server), Arc::clone(&radio));

        if let Ok(mut lock) = APP_STATE.lock() {
            *lock = Some(AppState { server, radio, lbl_radio, lbl_bridge, list_fans, edit_log, log_seq: u64::MAX });
        }
    }
}

/// Finds exactly one controller (our C3) and then brings everything up on its own:
/// radio link, USB/IP server, virtual ports, L-Connect service. Retries until it can.
fn spawn_startup(server: Arc<Mutex<VirtualDongleServer>>, radio: Arc<Mutex<Radio>>) {
    std::thread::spawn(move || {
        // Ghost ports from a crashed run would look like a second controller (and to L-Connect too)
        let _ = detach_all_virtual();
        let mut warned = false;
        let port = loop {
            match crate::hardware::c3_link::detect() {
                Ok(p) => break p,
                Err((msg, conflict)) => {
                    crate::traffic_log::log_event("GUI", "detect", &msg);
                    if conflict && !warned {
                        warned = true;
                        let m = to_wide(&msg);
                        unsafe {
                            let _ = MessageBoxW(None, PCWSTR(m.as_ptr()), w!("More than one controller"), MB_OK | MB_ICONWARNING);
                        }
                    }
                    *radio.lock().unwrap() = if conflict { Radio::Conflict(msg) } else { Radio::Searching(msg) };
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        };

        if let Ok(mut s) = server.lock() {
            let link = crate::hardware::c3_link::C3Link::start(&port, s.air_packet_sink());
            s.set_forwarder(move |pkt| link.forward(pkt));
            if let Err(e) = s.start("127.0.0.1:3240") {
                crate::traffic_log::log_event("GUI", "bridge", &format!("start failed: {}", e));
            }
        }
        *radio.lock().unwrap() = Radio::Running(port);
        if let Err(e) = attach_virtual() {
            crate::traffic_log::log_event("GUI", "usbip", &format!("attach failed: {}", e));
        }
        let _ = crate::hardware::start_service("LConnectService");
        let _ = crate::hardware::start_service("LConnectServiceWatcher");
    });
}

static TICK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PORTS_TEXT: Mutex<String> = Mutex::new(String::new());

unsafe fn on_timer_tick() {
    unsafe {
        let Ok(mut guard) = APP_STATE.lock() else { return };
        let Some(state) = guard.as_mut() else { return };
        let stats = state.server.lock().map(|s| s.get_stats()).unwrap_or_default();
        let radio = state.radio.lock().map(|r| r.clone()).unwrap_or(Radio::Searching(String::new()));

        set_text(state.lbl_radio, &match &radio {
            Radio::Searching(m) => format!("Waiting: {}", m),
            Radio::Conflict(m) => format!("BLOCKED: {}", m),
            Radio::Running(p) => format!("ESP32-C3 RF Master on {} (Lian Li 2.4 GHz network)", p),
        });

        if TICK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(3) {
            let ports = list_ports().map(|p| p.len()).unwrap_or(0);
            *PORTS_TEXT.lock().unwrap() = format!("USB/IP ports: {}", ports);
        }
        let lconnect = if stats.tx_connected && stats.rx_connected { "connected (TX+RX)" } else if stats.tx_connected { "connected (TX)" } else { "waiting" };
        set_text(state.lbl_bridge, &format!(
            "Virtual dongle: {} | L-Connect: {} | {} | commands: {} received, {} sent to radio",
            if stats.is_running { "running" } else { "stopped" }, lconnect, PORTS_TEXT.lock().unwrap(), stats.packets_received, stats.packets_forwarded,
        ));

        populate_fans(state.list_fans, &stats.air_nodes);

        let (seq, lines) = crate::traffic_log::recent_lines();
        if seq != state.log_seq {
            state.log_seq = seq;
            set_text(state.edit_log, &lines.join("\r\n"));
            let _ = send_msg(state.edit_log, WM_VSCROLL, SB_BOTTOM.0 as usize, 0);
        }
    }
}

fn mac(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect::<Vec<_>>().join(":")
}

unsafe fn populate_fans(list: HWND, nodes: &[crate::bridge::rf_protocol::AirNode]) {
    unsafe {
        let _ = send_msg(list, LVM_DELETEALLITEMS, 0, 0);
        for (row, n) in nodes.iter().filter(|n| n.last_heard.elapsed() < AIR_NODE_TTL).enumerate() {
            let a = &n.a;
            let fans = a[19].min(4) as usize;
            let rpm: Vec<String> = (0..fans).map(|i| (u16::from_be_bytes([a[28 + 2 * i] & 0x0F, a[29 + 2 * i]])).to_string()).collect();
            let pwm: Vec<String> = (0..fans).map(|i| a[36 + i].to_string()).collect();
            let master = if a[6..12] == [0; 6] { "none".to_string() } else { mac(&a[6..12]) };
            let cells = [mac(&a[0..6]), master, a[13].to_string(), a[19].to_string(), rpm.join(" / "), pwm.join(" / "), format!("{:.0} s", n.last_heard.elapsed().as_secs_f32())];
            for (col, text) in cells.iter().enumerate() {
                let mut w = to_wide(text);
                let item = LVITEMW { mask: LVIF_TEXT, iItem: row as i32, iSubItem: col as i32, pszText: PWSTR(w.as_mut_ptr()), ..Default::default() };
                let msg = if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
                let _ = send_msg(list, msg, row, &item as *const _ as isize);
            }
        }
    }
}

unsafe fn init_tray(hwnd: HWND) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY_ICON,
            hIcon: app_icon(),
            ..Default::default()
        };
        let tip = to_wide("Lian Li Controller (ESP32-C3)");
        let copy_len = tip.len().min(nid.szTip.len());
        nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

unsafe fn remove_tray(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

unsafe fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, IDM_TRAY_OPEN, w!("Open"));
        let auto = if crate::hardware::autostart::is_enabled() { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | auto, IDM_TRAY_AUTOSTART, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, IDM_TRAY_EXIT, w!("Exit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            IDM_TRAY_OPEN => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            IDM_TRAY_AUTOSTART => {
                if let Err(e) = crate::hardware::autostart::set_enabled(!crate::hardware::autostart::is_enabled()) {
                    let m = to_wide(&e);
                    let _ = MessageBoxW(Some(hwnd), PCWSTR(m.as_ptr()), w!("Start with Windows"), MB_OK | MB_ICONWARNING);
                }
            }
            IDM_TRAY_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}
