//! Esconde e devolve a barra de tarefas do Windows.
//!
//! Segurança: antes de mexer, o estado original da barra é salvo em
//! %LOCALAPPDATA%\Cais\barra-original.txt. Enquanto esse arquivo existir,
//! o Cais sabe que precisa devolver a barra (ao sair, ao desligar o PC, ou
//! na próxima abertura se ele tiver travado). `cais.exe --restaurar-barra`
//! faz isso na mão.

use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn arquivo() -> PathBuf {
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Cais");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("barra-original.txt")
}

// "Mostrar a barra em todos os monitores" (Configurações > Barra de tarefas).
// Escondida, a barra dos outros monitores ainda reaparece quando o mouse
// encosta embaixo; com a opção desligada ela nem existe. Só enquanto o Cais
// esconde a barra: o valor original fica no mesmo arquivo e volta ao sair.
const AVANCADO: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced");

fn barra_em_todos() -> Option<u32> {
    use windows::Win32::System::Registry::*;
    let mut v = 0u32;
    let mut tam = 4u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, AVANCADO, w!("MMTaskbarEnabled"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as _), Some(&mut tam))
            .is_ok()
            .then_some(v)
    }
}

fn definir_barra_em_todos(v: u32) {
    use windows::Win32::System::Registry::*;
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, AVANCADO, w!("MMTaskbarEnabled"), REG_DWORD.0, Some(&v as *const _ as _), 4);
        // Avisa o Explorer (é o que a tela de Configurações faz).
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("TraySettings").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
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

/// A bandeja de verdade (do Explorer). O Cais também tem uma janela com esse
/// nome (para receber os ícones da bandeja), que fica de fora.
pub fn bandeja_do_explorer() -> Option<HWND> {
    let mut anterior = HWND::default();
    unsafe {
        while let Ok(h) = FindWindowExW(None, anterior, w!("Shell_TrayWnd"), PCWSTR::null()) {
            if h.0.is_null() {
                break;
            }
            if !eh_do_cais(h) {
                return Some(h);
            }
            anterior = h;
        }
    }
    None
}

/// Barra principal + barras dos outros monitores.
fn bandejas() -> Vec<HWND> {
    let mut v = Vec::new();
    unsafe {
        if let Some(h) = bandeja_do_explorer() {
            v.push(h);
        }
        let mut anterior = HWND::default();
        while let Ok(h) = FindWindowExW(None, anterior, w!("Shell_SecondaryTrayWnd"), PCWSTR::null()) {
            if h.0.is_null() {
                break;
            }
            v.push(h);
            anterior = h;
        }
    }
    v
}

fn estado(principal: HWND) -> u32 {
    let mut d = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: principal,
        ..Default::default()
    };
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

fn definir_estado(principal: HWND, s: u32) {
    let mut d = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: principal,
        lParam: LPARAM(s as isize),
        ..Default::default()
    };
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// Liga o "ocultar automaticamente" da barra (para liberar o espaço) e
/// esconde as janelas da barra (para ela não aparecer ao encostar o mouse).
pub fn esconder() {
    let bs = bandejas();
    let Some(&principal) = bs.first() else {
        return;
    };
    let arq = arquivo();
    if !arq.exists() {
        // Linha 1: estado da barra. Linha 2: barra em todos os monitores.
        let mm = barra_em_todos().map_or(String::new(), |v| v.to_string());
        let _ = std::fs::write(&arq, format!("{}\n{mm}", estado(principal)));
    }
    definir_estado(principal, ABS_AUTOHIDE);
    if barra_em_todos() == Some(1) && bs.len() > 1 {
        // Arquivo de uma versão anterior (só a linha 1): guarda o valor
        // antes de mudar, para devolver ao sair.
        if let Ok(t) = std::fs::read_to_string(&arq) {
            if t.lines().nth(1).map_or(true, |l| l.trim().is_empty()) {
                let primeira = t.lines().next().unwrap_or("0").trim().to_string();
                let _ = std::fs::write(&arq, format!("{primeira}\n1"));
            }
        }
        definir_barra_em_todos(0);
    }
    esconder_janelas();
}

pub fn esconder_janelas() {
    for h in bandejas() {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// Devolve a barra exatamente como estava (só se foi o Cais que escondeu).
pub fn mostrar() {
    let arq = arquivo();
    let Ok(t) = std::fs::read_to_string(&arq) else {
        return;
    };
    let mut linhas = t.lines();
    let original: u32 = linhas.next().unwrap_or("").trim().parse().unwrap_or(0);
    if let Some(mm) = linhas.next().and_then(|l| l.trim().parse::<u32>().ok()) {
        if barra_em_todos() != Some(mm) {
            definir_barra_em_todos(mm);
        }
    }
    let bs = bandejas();
    if let Some(&principal) = bs.first() {
        definir_estado(principal, original);
    }
    for h in bs {
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }
    let _ = std::fs::remove_file(arq);
}

pub fn eh_bandeja(h: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    let nome = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    (nome == "Shell_TrayWnd" || nome == "Shell_SecondaryTrayWnd") && !eh_do_cais(h)
}
