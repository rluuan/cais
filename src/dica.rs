//! Balão de dica (o "tooltip" do próprio Windows), posicionado à mão: o
//! nome de um ícone da bandeja quando o mouse para em cima dele.

use windows::core::{w, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Balao {
    janela: HWND,
    dono: HWND,
    /// Texto atual (o balão guarda o ponteiro: precisa continuar vivo).
    texto: std::cell::RefCell<Vec<u16>>,
}

impl Drop for Balao {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.janela);
        }
    }
}

impl Balao {
    pub fn new(dono: HWND) -> Option<Self> {
        unsafe {
            let ini = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_WIN95_CLASSES };
            let _ = InitCommonControlsEx(&ini);
            let janela = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("tooltips_class32"),
                None,
                WINDOW_STYLE(WS_POPUP.0 | TTS_NOPREFIX | TTS_ALWAYSTIP),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                dono,
                None,
                None,
                None,
            )
            .ok()?;
            let b = Balao { janela, dono, texto: std::cell::RefCell::new(vec![0]) };
            let mut ti = b.info();
            SendMessageW(janela, TTM_ADDTOOLW, WPARAM(0), LPARAM(&mut ti as *mut _ as isize));
            SendMessageW(janela, TTM_SETMAXTIPWIDTH, WPARAM(0), LPARAM(360));
            Some(b)
        }
    }

    fn info(&self) -> TTTOOLINFOW {
        TTTOOLINFOW {
            // Tamanho da versão antiga da estrutura: funciona com qualquer
            // versão da biblioteca de controles.
            cbSize: (std::mem::size_of::<TTTOOLINFOW>() - std::mem::size_of::<usize>()) as u32,
            uFlags: TTF_TRACK | TTF_ABSOLUTE,
            hwnd: self.dono,
            uId: 1,
            lpszText: PWSTR(self.texto.borrow_mut().as_mut_ptr()),
            ..Default::default()
        }
    }

    /// Mostra `texto` centralizado em x, com o topo em y (px da tela).
    pub fn mostrar(&self, texto: &str, x: i32, y: i32) {
        *self.texto.borrow_mut() = texto.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let mut ti = self.info();
            SendMessageW(self.janela, TTM_UPDATETIPTEXTW, WPARAM(0), LPARAM(&mut ti as *mut _ as isize));
            SendMessageW(self.janela, TTM_TRACKACTIVATE, WPARAM(1), LPARAM(&mut ti as *mut _ as isize));
            // Centraliza pela largura que o balão escolheu.
            let mut r = RECT::default();
            let _ = GetWindowRect(self.janela, &mut r);
            let l = r.right - r.left;
            let pos = ((y as u32 & 0xffff) << 16) | ((x - l / 2) as u32 & 0xffff);
            SendMessageW(self.janela, TTM_TRACKPOSITION, WPARAM(0), LPARAM(pos as isize));
        }
    }

    pub fn esconder(&self) {
        unsafe {
            let mut ti = self.info();
            SendMessageW(self.janela, TTM_TRACKACTIVATE, WPARAM(0), LPARAM(&mut ti as *mut _ as isize));
        }
    }
}
