//! Teste: restaurar uma janela invisível (camada com alfa 0) enquanto uma
//! miniatura DWM dela fica na tela, e só mostrar a janela depois.
//! Uso: teste_alfa <hwnd hex> [alfa|normal]
use std::time::Instant;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn proc_(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let alvo = HWND(i64::from_str_radix(a[1].trim_start_matches("0x"), 16).unwrap() as _);
    let alfa = a.get(2).map_or(true, |s| s == "alfa");
    unsafe {
        let inst = GetModuleHandleW(None).unwrap();
        let cls = w!("CaisTesteAlfa");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: inst.into(),
            lpszClassName: cls,
            hbrBackground: HBRUSH(GetStockObject(GRAY_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let w = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            cls,
            w!(""),
            WS_POPUP,
            4500,
            700,
            1000,
            600,
            None,
            None,
            inst,
            None,
        )
        .unwrap();
        let _ = ShowWindow(w, SW_SHOWNOACTIVATE);
        let th = DwmRegisterThumbnail(w, alvo).unwrap();
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: RECT { left: 0, top: 0, right: 1000, bottom: 600 },
            opacity: 255,
            fVisible: TRUE,
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(th, &props);
        let ex_original = GetWindowLongPtrW(alvo, GWL_EXSTYLE);
        let t0 = Instant::now();
        let mut passo = 0;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let ms = t0.elapsed().as_millis();
            if passo == 0 && ms >= 500 {
                if alfa {
                    SetWindowLongPtrW(alvo, GWL_EXSTYLE, ex_original | WS_EX_LAYERED.0 as isize);
                    let r = SetLayeredWindowAttributes(alvo, COLORREF(0), 0, LWA_ALPHA);
                    eprintln!("{ms} ms: alfa 0 ({r:?})");
                }
                let _ = ShowWindow(alvo, SW_RESTORE);
                eprintln!("{} ms: restaurada", t0.elapsed().as_millis());
                passo = 1;
            }
            if passo == 1 && ms >= 1000 {
                if alfa {
                    let _ = SetLayeredWindowAttributes(alvo, COLORREF(0), 255, LWA_ALPHA);
                    SetWindowLongPtrW(alvo, GWL_EXSTYLE, ex_original);
                    let _ = SetWindowPos(alvo, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                    eprintln!("{} ms: visivel de novo", t0.elapsed().as_millis());
                }
                passo = 2;
            }
            if passo == 2 && ms >= 1800 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let _ = DwmUnregisterThumbnail(th);
        let _ = DestroyWindow(w);
        eprintln!("ex final igual ao original: {}", GetWindowLongPtrW(alvo, GWL_EXSTYLE) == ex_original);
    }
}
