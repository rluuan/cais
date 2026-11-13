//! Áreas de trabalho virtuais (Win+Ctrl+D): quantas existem e em qual você
//! está, para a barra do topo mostrar os pontinhos. O Windows guarda isso
//! no registro do seu usuário; uma thread espera ele avisar que mudou
//! (RegNotifyChangeKeyValue), sem ficar consultando. Só leitura.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_AREAS: u32 = WM_APP + 32;
const CHAVE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops");
static LIGADO: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct Areas {
    /// Índice da área atual (0 = primeira) e quantas existem.
    pub atual: usize,
    pub total: usize,
}

fn ler_binario(nome: PCWSTR) -> Vec<u8> {
    unsafe {
        let mut tam = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, CHAVE, nome, RRF_RT_REG_BINARY, None, None, Some(&mut tam)).is_err() || tam == 0 {
            return Vec::new();
        }
        let mut v = vec![0u8; tam as usize];
        if RegGetValueW(HKEY_CURRENT_USER, CHAVE, nome, RRF_RT_REG_BINARY, None, Some(v.as_mut_ptr() as _), Some(&mut tam)).is_err() {
            return Vec::new();
        }
        v.truncate(tam as usize);
        v
    }
}

/// (atual, total) a partir da lista de GUIDs (16 bytes cada) e do atual.
pub fn calcular(lista: &[u8], atual: &[u8]) -> (usize, usize) {
    let ids: Vec<&[u8]> = lista.chunks_exact(16).collect();
    let i = ids.iter().position(|g| *g == atual).unwrap_or(0);
    (i, ids.len().max(1))
}

impl Areas {
    pub fn ler(&mut self) {
        let (a, t) = calcular(&ler_binario(w!("VirtualDesktopIDs")), &ler_binario(w!("CurrentVirtualDesktop")));
        self.atual = a;
        self.total = t;
    }

    /// Começa a vigiar o registro (uma vez) e avisa `aviso` a cada mudança.
    pub fn ligar(&mut self, aviso: HWND) {
        self.ler();
        if LIGADO.swap(true, Ordering::Relaxed) {
            return;
        }
        let h = aviso.0 as isize;
        std::thread::spawn(move || unsafe {
            let mut chave = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, CHAVE, 0, KEY_NOTIFY, &mut chave).is_err() {
                return;
            }
            let Ok(evento) = CreateEventW(None, false, false, None) else { return };
            loop {
                if RegNotifyChangeKeyValue(chave, false, REG_NOTIFY_CHANGE_LAST_SET, evento, true).is_err() {
                    break;
                }
                WaitForSingleObject(evento, INFINITE);
                let _ = PostMessageW(HWND(h as *mut _), WM_AREAS, WPARAM(0), LPARAM(0));
            }
        });
    }
}

/// Vai para a área `destino` apertando Win+Ctrl+seta quantas vezes precisar.
pub fn ir_para(atual: usize, destino: usize) {
    if destino == atual {
        return;
    }
    let seta = if destino > atual { VK_RIGHT } else { VK_LEFT };
    let vezes = destino.abs_diff(atual);
    let tecla = |vk: VIRTUAL_KEY, sobe: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if sobe { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }
                    | if vk == VK_LEFT || vk == VK_RIGHT || vk == VK_LWIN { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut v = vec![tecla(VK_LWIN, false), tecla(VK_LCONTROL, false)];
    for _ in 0..vezes {
        v.push(tecla(seta, false));
        v.push(tecla(seta, true));
    }
    v.push(tecla(VK_LCONTROL, true));
    v.push(tecla(VK_LWIN, true));
    unsafe {
        SendInput(&v, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn acha_a_area_atual() {
        let a = [1u8; 16];
        let b = [2u8; 16];
        let c = [3u8; 16];
        let lista: Vec<u8> = [a, b, c].concat();
        assert_eq!(calcular(&lista, &b), (1, 3));
        assert_eq!(calcular(&lista, &c), (2, 3));
        assert_eq!(calcular(&[], &[]), (0, 1));
    }
}
