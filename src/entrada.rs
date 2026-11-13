//! Ganchos de teclado e mouse do sistema, numa thread só deles (o Windows
//! exige resposta rápida; a thread da tela pode estar desenhando).
//!
//! - Teclado: Alt+Tab vira o alternador de apps do Cais (se ligado).
//! - Mouse: encostar num canto da tela dispara a ação do canto ativo.
//!
//! Os ganchos só existem enquanto o recurso está ligado: desligado, nenhum
//! evento passa por aqui. Nada é gravado; só avisa a janela do Cais.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Alternador: wp = EV_* abaixo.
pub const WM_ALTERNADOR: u32 = WM_APP + 30;
/// Canto ativo: wp = índice do canto (0 sup. esq., 1 sup. dir., 2 inf. esq., 3 inf. dir.).
pub const WM_CANTO: u32 = WM_APP + 31;

pub const EV_ABRIR: usize = 0;
pub const EV_PROXIMO: usize = 1;
pub const EV_ANTERIOR: usize = 2;
pub const EV_CONFIRMAR: usize = 3;
pub const EV_CANCELAR: usize = 4;

static AVISO: AtomicIsize = AtomicIsize::new(0);
static THREAD: AtomicU32 = AtomicU32::new(0);
static ALTERNADOR: AtomicBool = AtomicBool::new(false);
static CANTOS: AtomicBool = AtomicBool::new(false);
/// O alternador está aberto (Alt ainda segurado).
static ABERTO: AtomicBool = AtomicBool::new(false);
/// Último canto disparado (-1 = nenhum); só dispara de novo depois de sair.
static CANTO_ATUAL: AtomicIsize = AtomicIsize::new(-1);

const WM_REVER: u32 = WM_APP + 1;
/// Tecla sem uso, mandada para o Alt solto não abrir o menu do app.
const VK_NEUTRA: u16 = 0xE8;

/// Liga a thread (uma vez) e ajusta quais ganchos ficam ativos.
pub fn configurar(aviso: HWND, alternador: bool, cantos: bool) {
    AVISO.store(aviso.0 as isize, Ordering::Relaxed);
    ALTERNADOR.store(alternador, Ordering::Relaxed);
    CANTOS.store(cantos, Ordering::Relaxed);
    let t = THREAD.load(Ordering::Relaxed);
    if t == 0 {
        if !alternador && !cantos {
            return;
        }
        std::thread::spawn(laco);
    } else {
        unsafe {
            let _ = PostThreadMessageW(t, WM_REVER, WPARAM(0), LPARAM(0));
        }
    }
}

fn avisar(msg: u32, wp: usize) {
    let h = HWND(AVISO.load(Ordering::Relaxed) as *mut _);
    unsafe {
        let _ = PostMessageW(h, msg, WPARAM(wp), LPARAM(0));
    }
}

fn laco() {
    unsafe {
        THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
        let mut teclado: Option<HHOOK> = None;
        let mut mouse: Option<HHOOK> = None;
        let rever = |teclado: &mut Option<HHOOK>, mouse: &mut Option<HHOOK>| {
            let quer_t = ALTERNADOR.load(Ordering::Relaxed);
            let quer_m = CANTOS.load(Ordering::Relaxed);
            match (quer_t, teclado.is_some()) {
                (true, false) => *teclado = SetWindowsHookExW(WH_KEYBOARD_LL, Some(gancho_teclado), None, 0).ok(),
                (false, true) => {
                    let _ = UnhookWindowsHookEx(teclado.take().unwrap());
                }
                _ => {}
            }
            match (quer_m, mouse.is_some()) {
                (true, false) => *mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(gancho_mouse), None, 0).ok(),
                (false, true) => {
                    let _ = UnhookWindowsHookEx(mouse.take().unwrap());
                }
                _ => {}
            }
        };
        rever(&mut teclado, &mut mouse);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_REVER {
                rever(&mut teclado, &mut mouse);
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}

fn tecla_baixa(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

fn mandar_neutra() {
    let mk = |sobe: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_NEUTRA),
                wScan: 0,
                dwFlags: if sobe { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[mk(false), mk(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn gancho_teclado(codigo: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if codigo >= 0 && ALTERNADOR.load(Ordering::Relaxed) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injetada = k.flags.0 & LLKHF_INJECTED.0 != 0;
        let desceu = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let subiu = matches!(wp.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        let vk = VIRTUAL_KEY(k.vkCode as u16);
        let alt = k.flags.0 & LLKHF_ALTDOWN.0 != 0 || tecla_baixa(VK_MENU);
        if !injetada {
            if vk == VK_TAB && desceu && alt {
                let shift = tecla_baixa(VK_SHIFT);
                if !ABERTO.swap(true, Ordering::Relaxed) {
                    mandar_neutra();
                    avisar(WM_ALTERNADOR, EV_ABRIR);
                }
                avisar(WM_ALTERNADOR, if shift { EV_ANTERIOR } else { EV_PROXIMO });
                return LRESULT(1);
            }
            if vk == VK_TAB && subiu && ABERTO.load(Ordering::Relaxed) {
                return LRESULT(1);
            }
            if ABERTO.load(Ordering::Relaxed) {
                match vk {
                    VK_ESCAPE if desceu => {
                        ABERTO.store(false, Ordering::Relaxed);
                        avisar(WM_ALTERNADOR, EV_CANCELAR);
                        return LRESULT(1);
                    }
                    VK_LEFT | VK_RIGHT if desceu => {
                        avisar(WM_ALTERNADOR, if vk == VK_LEFT { EV_ANTERIOR } else { EV_PROXIMO });
                        return LRESULT(1);
                    }
                    VK_LMENU | VK_RMENU | VK_MENU if subiu => {
                        ABERTO.store(false, Ordering::Relaxed);
                        avisar(WM_ALTERNADOR, EV_CONFIRMAR);
                    }
                    _ => {}
                }
            }
        }
    }
    CallNextHookEx(None, codigo, wp, lp)
}

unsafe extern "system" fn gancho_mouse(codigo: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if codigo >= 0 && wp.0 as u32 == WM_MOUSEMOVE && CANTOS.load(Ordering::Relaxed) {
        let m = &*(lp.0 as *const MSLLHOOKSTRUCT);
        olhar_canto(m.pt);
    }
    CallNextHookEx(None, codigo, wp, lp)
}

/// Encostou num canto do monitor? Dispara uma vez; rearma ao se afastar.
fn olhar_canto(p: POINT) {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let m = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(m, &mut info).as_bool() {
            return;
        }
    }
    let r = info.rcMonitor;
    let canto = canto_de(p, r, 1);
    let atual = CANTO_ATUAL.load(Ordering::Relaxed);
    match canto {
        Some(c) if atual != c as isize => {
            CANTO_ATUAL.store(c as isize, Ordering::Relaxed);
            avisar(WM_CANTO, c);
        }
        None if atual >= 0 && canto_de(p, r, 12).is_none() => CANTO_ATUAL.store(-1, Ordering::Relaxed),
        _ => {}
    }
}

/// Em qual canto de `r` o ponto está (com tolerância de `tol` px).
pub fn canto_de(p: POINT, r: RECT, tol: i32) -> Option<usize> {
    let esq = p.x < r.left + tol;
    let dir = p.x >= r.right - tol;
    let cima = p.y < r.top + tol;
    let baixo = p.y >= r.bottom - tol;
    match (esq, dir, cima, baixo) {
        (true, _, true, _) => Some(0),
        (_, true, true, _) => Some(1),
        (true, _, _, true) => Some(2),
        (_, true, _, true) => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn cantos_da_tela() {
        let r = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        assert_eq!(canto_de(POINT { x: 0, y: 0 }, r, 1), Some(0));
        assert_eq!(canto_de(POINT { x: 1919, y: 0 }, r, 1), Some(1));
        assert_eq!(canto_de(POINT { x: 0, y: 1079 }, r, 1), Some(2));
        assert_eq!(canto_de(POINT { x: 1919, y: 1079 }, r, 1), Some(3));
        assert_eq!(canto_de(POINT { x: 500, y: 0 }, r, 1), None);
        assert_eq!(canto_de(POINT { x: 5, y: 5 }, r, 1), None);
        assert_eq!(canto_de(POINT { x: 5, y: 5 }, r, 12), Some(0));
    }
}
