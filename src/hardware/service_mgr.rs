use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::thread;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::System::Services::*;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub fn is_service_running(service_name: &str) -> bool {
    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let s_name = to_wide(service_name);
        let service = match OpenServiceW(scm, PCWSTR(s_name.as_ptr()), SERVICE_QUERY_STATUS) {
            Ok(h) => h,
            Err(_) => {
                let _ = CloseServiceHandle(scm);
                return false;
            }
        };

        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut bytes_needed = 0;
        let buf = std::slice::from_raw_parts_mut(
            &mut status as *mut _ as *mut u8,
            std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
        );
        let res = QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            Some(buf),
            &mut bytes_needed,
        );

        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);

        if res.is_ok() {
            status.dwCurrentState == SERVICE_RUNNING
        } else {
            false
        }
    }
}

pub fn start_service(service_name: &str) -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("Failed to open SC Manager: {}", e))?;
        let s_name = to_wide(service_name);
        let service = OpenServiceW(scm, PCWSTR(s_name.as_ptr()), SERVICE_START | SERVICE_QUERY_STATUS)
            .map_err(|e| {
                let _ = CloseServiceHandle(scm);
                format!("Failed to open service {}: {}", service_name, e)
            })?;

        let _ = StartServiceW(service, None);

        // Wait up to 3 seconds for service to start
        for _ in 0..15 {
            let mut status_ex = SERVICE_STATUS_PROCESS::default();
            let mut bytes_needed = 0;
            let buf = std::slice::from_raw_parts_mut(
                &mut status_ex as *mut _ as *mut u8,
                std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
            );
            if QueryServiceStatusEx(
                service,
                SC_STATUS_PROCESS_INFO,
                Some(buf),
                &mut bytes_needed,
            ).is_ok()
                && status_ex.dwCurrentState == SERVICE_RUNNING {
                    break;
                }
            thread::sleep(Duration::from_millis(200));
        }

        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);
        Ok(())
    }
}
