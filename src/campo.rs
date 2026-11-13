//! Campo de texto de uma linha (busca e Launchpad): digitar, apagar,
//! apagar palavra, mover o cursor, colar e "tudo selecionado".

use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

const CF_UNICODETEXT: u32 = 13;

#[derive(Default)]
pub struct Campo {
    pub texto: Vec<char>,
    pub cursor: usize,
    /// Tudo selecionado: o próximo caractere substitui o texto.
    pub tudo: bool,
}

pub enum Tecla {
    Nada,
    Mudou,
    Moveu,
}

impl Campo {
    pub fn string(&self) -> String {
        self.texto.iter().collect()
    }

    pub fn limpar(&mut self) {
        self.texto.clear();
        self.cursor = 0;
        self.tudo = false;
    }

    fn apagar_selecao(&mut self) -> bool {
        if self.tudo && !self.texto.is_empty() {
            self.limpar();
            return true;
        }
        self.tudo = false;
        false
    }

    pub fn inserir(&mut self, s: &str) {
        self.apagar_selecao();
        for c in s.chars().filter(|c| !c.is_control()) {
            self.texto.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// WM_CHAR: caracteres imprimíveis (acentos já vêm prontos do Windows).
    pub fn caractere(&mut self, c: u32) -> Tecla {
        match char::from_u32(c) {
            Some(ch) if !ch.is_control() => {
                self.inserir(&ch.to_string());
                Tecla::Mudou
            }
            _ => Tecla::Nada,
        }
    }

    /// WM_KEYDOWN: edição e movimento do cursor.
    pub fn tecla(&mut self, vk: u16) -> Tecla {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let vk = VIRTUAL_KEY(vk);
        match vk {
            VK_BACK => {
                if self.apagar_selecao() {
                    return Tecla::Mudou;
                }
                if self.cursor == 0 {
                    return Tecla::Nada;
                }
                let ini = if ctrl { self.inicio_da_palavra() } else { self.cursor - 1 };
                self.texto.drain(ini..self.cursor);
                self.cursor = ini;
                Tecla::Mudou
            }
            VK_DELETE => {
                if self.apagar_selecao() {
                    return Tecla::Mudou;
                }
                if self.cursor < self.texto.len() {
                    self.texto.remove(self.cursor);
                    return Tecla::Mudou;
                }
                Tecla::Nada
            }
            VK_LEFT => {
                self.tudo = false;
                self.cursor = if ctrl { self.inicio_da_palavra() } else { self.cursor.saturating_sub(1) };
                Tecla::Moveu
            }
            VK_RIGHT => {
                self.tudo = false;
                self.cursor = (self.cursor + 1).min(self.texto.len());
                Tecla::Moveu
            }
            VK_HOME => {
                self.tudo = false;
                self.cursor = 0;
                Tecla::Moveu
            }
            VK_END => {
                self.tudo = false;
                self.cursor = self.texto.len();
                Tecla::Moveu
            }
            VIRTUAL_KEY(0x41) if ctrl => {
                // Ctrl+A
                self.tudo = !self.texto.is_empty();
                self.cursor = self.texto.len();
                Tecla::Moveu
            }
            VIRTUAL_KEY(0x56) if ctrl => {
                // Ctrl+V
                match colar() {
                    Some(s) => {
                        self.inserir(&s);
                        Tecla::Mudou
                    }
                    None => Tecla::Nada,
                }
            }
            _ => Tecla::Nada,
        }
    }

    fn inicio_da_palavra(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.texto[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && self.texto[i - 1] != ' ' {
            i -= 1;
        }
        i
    }
}

fn colar() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let r = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut n = 0;
            while *p.add(n) != 0 && n < 4096 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(g);
            Some(s.lines().next().unwrap_or("").to_string())
        })();
        let _ = CloseClipboard();
        r
    }
}

/// Copia um texto para a área de transferência (resultado da conta).
pub fn copiar(texto: &str) {
    let w: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(g) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(g);
                let _ = SetClipboardData(CF_UNICODETEXT, HANDLE(g.0));
            }
        }
        let _ = CloseClipboard();
    }
}
