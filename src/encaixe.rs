//! Janelas maximizadas (ou encaixadas na borda) que ficaram por baixo da
//! barra do topo, do dock ou da faixa da esquerda voltam para a área livre.
//!
//! O Windows respeita o espaço reservado só nas janelas que você maximiza
//! depois; as que já estavam maximizadas quando a reserva mudou (o Cais
//! reiniciou, o dock deixou de se ocultar) ficam por baixo. Janelas soltas,
//! que você posicionou à mão, não são mexidas.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Timer (na janela do dock) que confere todas as janelas pouco depois de
/// uma reserva mudar.
pub const TIMER_ENCAIXE: usize = 8;

static AVISAR: AtomicIsize = AtomicIsize::new(0);
/// Algum app está em tela cheia (vídeo, jogo): nada é mexido.
pub static TELA_CHEIA: AtomicBool = AtomicBool::new(false);
/// Só depois que o Cais está de pé (no modo de medição, nunca).
static ATIVO: AtomicBool = AtomicBool::new(false);

/// Janela que recebe o timer (o dock). Confere tudo logo em seguida.
pub fn definir_aviso(h: HWND) {
    AVISAR.store(h.0 as isize, Ordering::Relaxed);
    ATIVO.store(true, Ordering::Relaxed);
    agendar();
}

/// Uma reserva mudou: confere as janelas daqui a pouco (o Windows precisa
/// de um instante para atualizar a área livre).
pub fn agendar() {
    let h = HWND(AVISAR.load(Ordering::Relaxed) as *mut _);
    if !h.0.is_null() {
        unsafe {
            SetTimer(h, TIMER_ENCAIXE, 250, None);
        }
    }
}

/// (retângulo da janela, parte visível dela). No Windows 11 a janela tem
/// uma borda invisível de alguns pixels em volta.
pub fn moldura(h: HWND) -> Option<(RECT, RECT)> {
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let mut v = RECT::default();
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut v as *mut _ as _, std::mem::size_of::<RECT>() as u32).is_err() {
            v = r;
        }
        Some((r, v))
    }
}

/// Põe a parte visível da janela exatamente em `alvo` (px da tela). Não
/// espera o app responder (um app travado não trava o Cais).
pub fn posicionar_visivel(h: HWND, alvo: RECT) {
    let Some((r, v)) = moldura(h) else { return };
    let x = alvo.left - (v.left - r.left);
    let y = alvo.top - (v.top - r.top);
    let dir = alvo.right + (r.right - v.right);
    let base = alvo.bottom + (r.bottom - v.bottom);
    unsafe {
        let _ = SetWindowPos(
            h,
            None,
            x,
            y,
            dir - x,
            base - y,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        );
    }
}

fn eh_do_cais(h: HWND) -> bool {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    pid == std::process::id()
}

/// Confere uma janela e, se ela invadiu um espaço reservado, encaixa.
pub fn ajustar(h: HWND) {
    if h.0.is_null() || !ATIVO.load(Ordering::Relaxed) || TELA_CHEIA.load(Ordering::Relaxed) || eh_do_cais(h) {
        return;
    }
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return;
        }
        let estilo = GetWindowLongW(h, GWL_STYLE) as u32;
        if estilo & WS_CHILD.0 != 0 {
            return;
        }
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let (m, w) = crate::janela::info_monitor(mon);
        if m == w {
            return; // nada reservado neste monitor
        }
        let Some((r, v)) = moldura(h) else { return };
        let dpi = GetDpiForWindow(h).max(96) as i32;
        let tol = 12 * dpi / 96;
        // Cobre o monitor inteiro sem moldura: tela cheia, não mexe.
        let cobre = v.left <= m.left + 2 && v.top <= m.top + 2 && v.right >= m.right - 2 && v.bottom >= m.bottom - 2;
        if IsZoomed(h).as_bool() {
            if cobre && estilo & WS_CAPTION.0 != WS_CAPTION.0 {
                return;
            }
            let invade = [r.left < w.left - tol, r.top < w.top - tol, r.right > w.right + tol, r.bottom > w.bottom + tol];
            if !invade.iter().any(|&b| b) {
                return;
            }
            // Maximizada: as bordas ficam um pouco para fora da área livre
            // (a mesma margem dos lados que não invadiram).
            let margens = [w.left - r.left, w.top - r.top, r.right - w.right, r.bottom - w.bottom];
            let margem = margens
                .iter()
                .zip(invade)
                .filter(|(_, inv)| !inv)
                .map(|(m, _)| *m)
                .filter(|m| (0..=tol).contains(m))
                .max()
                .unwrap_or(0);
            let alvo = RECT { left: w.left - margem, top: w.top - margem, right: w.right + margem, bottom: w.bottom + margem };
            let _ = SetWindowPos(
                h,
                None,
                alvo.left,
                alvo.top,
                alvo.right - alvo.left,
                alvo.bottom - alvo.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
            );
            return;
        }
        // Tela cheia, ou janela "guardada" fora da tela (alguns apps se
        // escondem em -32000): não mexe.
        let dentro = v.left >= m.left - 2 && v.top >= m.top - 2 && v.right <= m.right + 2 && v.bottom <= m.bottom + 2;
        if cobre || !dentro {
            return;
        }
        // Encaixada (meia tela, canto): encosta na borda do monitor e
        // entrou na faixa reservada. Só o lado que invadiu encolhe.
        let perto = |a: i32, b: i32| (a - b).abs() <= 2;
        let mut alvo = v;
        if w.top > m.top && perto(v.top, m.top) {
            alvo.top = w.top;
        }
        if w.bottom < m.bottom && perto(v.bottom, m.bottom) {
            alvo.bottom = w.bottom;
        }
        if w.left > m.left && perto(v.left, m.left) {
            alvo.left = w.left;
        }
        if w.right < m.right && perto(v.right, m.right) {
            alvo.right = w.right;
        }
        if alvo != v && alvo.right - alvo.left > 120 && alvo.bottom - alvo.top > 100 {
            posicionar_visivel(h, alvo);
        }
    }
}

/// Confere todas as janelas de apps.
pub fn ajustar_todas() {
    for (h, _) in crate::apps::enumerar_janelas() {
        ajustar(h);
    }
}

/// cais.exe --testar-encaixe: só mostra (não mexe) as janelas que
/// invadem espaço reservado, e as áreas dos layouts de cada monitor.
pub fn testar() {
    for m in crate::janela::monitores() {
        let (mon, w) = crate::janela::info_monitor(m);
        println!("monitor {:?}: tela {:?} área livre {:?}", m, mon, w);
        crate::layouts::configurar(true, 3, 3, 1);
        for (i, z) in crate::layouts::zonas(m).iter().enumerate() {
            println!("  área 3x2 #{i}: {},{} {}x{}", z.left, z.top, z.right - z.left, z.bottom - z.top);
        }
    }
    for (h, exe) in crate::apps::enumerar_janelas() {
        unsafe {
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                continue;
            }
            let Some((r, v)) = moldura(h) else { continue };
            let (_, w) = crate::janela::info_monitor(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST));
            let z = IsZoomed(h).as_bool();
            let fora = v.top < w.top || v.bottom > w.bottom || v.left < w.left || v.right > w.right;
            println!(
                "{exe:28} {} janela ({},{},{},{}) visível ({},{},{},{}){}",
                if z { "MAXIMIZADA" } else { "solta     " },
                r.left, r.top, r.right, r.bottom, v.left, v.top, v.right, v.bottom,
                if fora { "  <- invade" } else { "" }
            );
        }
    }
}
