//! "Arredondar todas as janelas": pede ao Windows 11 os cantos arredondados
//! nas janelas dos outros apps, inclusive nas que desligaram isso.
//!
//! O Windows aceita o pedido vindo de outro programa (menos para janelas de
//! administrador, quando o Cais não é administrador). Janela maximizada ou
//! encaixada continua reta: é o próprio Windows que decide.
//!
//! O pedido vale enquanto a janela existir; por isso o Cais repete em cada
//! janela que aparece ou volta ao primeiro plano. Ao desligar, as janelas já
//! arredondadas ficam assim até fechar (não dá para saber o que o app tinha
//! pedido antes).

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE,
    WS_CAPTION, WS_CHILD, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};

static LIGADO: AtomicBool = AtomicBool::new(false);

pub fn ligar(sim: bool) {
    let antes = LIGADO.swap(sim, Ordering::Relaxed);
    if sim && !antes {
        todas();
    }
}

/// Só janelas "de verdade": com moldura ou barra de título, fora as de
/// ferramenta e as filhas. Menus, dicas e popups ficam como o app quer.
fn serve(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        let estilo = GetWindowLongW(h, GWL_STYLE) as u32;
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if estilo & WS_CHILD.0 != 0 || ex & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        if estilo & (WS_CAPTION.0 | WS_THICKFRAME.0) == 0 {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        pid != std::process::id()
    }
}

/// Arredonda uma janela, se a opção estiver ligada.
pub fn aplicar(h: HWND) {
    if !LIGADO.load(Ordering::Relaxed) || h.0.is_null() || !serve(h) {
        return;
    }
    let canto = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            h,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &canto as *const _ as _,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// Todas as janelas abertas (ao ligar a opção ou ao iniciar o Cais).
pub fn todas() {
    unsafe extern "system" fn cada(h: HWND, _: LPARAM) -> BOOL {
        aplicar(h);
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(cada), LPARAM(0));
    }
}
