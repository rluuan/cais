//! Alternador de apps (Alt+Tab no estilo do Cais): os ícones dos apps
//! abertos, do mais recente ao mais antigo, num painel de vidro. Tab avança,
//! Shift+Tab volta, soltar o Alt troca para o app escolhido, Esc cancela.
//! Um Alt+Tab rápido troca direto, sem o painel piscar na tela.

use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::Vidro;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Timer que mostra o painel se o Alt continuar segurado.
pub const TIMER_ALTERNADOR: usize = 31;
const ICONE: f32 = 84.0;
const CELULA: f32 = 112.0;
const FOLGA: f32 = 18.0;
const ALTURA: f32 = 150.0;

pub struct App {
    pub nome: String,
    pub icone: Option<ID2D1Bitmap1>,
    /// Janela mais à frente do app (a que vem ao trocar).
    pub janela: HWND,
}

pub struct Alternador {
    sup: Superficie,
    _vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    formato: IDWriteTextFormat,
    apps: Vec<App>,
    pub escolhido: usize,
    pub visivel: bool,
    pub sujo: bool,
}

impl Drop for Alternador {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

impl Alternador {
    pub fn abrir(gpu: &Gpu, apps: Vec<App>) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)?;
        janela::preparar_vidro(hwnd);
        let e = janela::escala(hwnd);
        let (mon, _) = janela::monitor_do_cursor();
        let n = apps.len().max(1) as f32;
        let largura_max = (mon.right - mon.left) as f32 / e - 80.0;
        let l = (FOLGA * 2.0 + n * CELULA).min(largura_max);
        let (lp, ap) = ((l * e).round() as i32, (ALTURA * e).round() as i32);
        let x = mon.left + (mon.right - mon.left - lp) / 2;
        let y = mon.top + (mon.bottom - mon.top - ap) / 2;
        janela::posicionar(hwnd, x, y, lp, ap);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 22.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, crate::tema::Tinta::Painel)?;
        vidro.definir(0.0, 0.0, lp as f32, ap as f32, 22.0 * e)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let formato = gpu.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0,
                crate::idioma::localidade(),
            )?;
            formato.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            formato.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(Self { sup, _vidro: vidro, pincel, formato, apps, escolhido: 0, visivel: false, sujo: true })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn mostrar(&mut self) {
        if !self.visivel {
            self.visivel = true;
            unsafe {
                let _ = ShowWindow(self.sup.hwnd, SW_SHOWNOACTIVATE);
            }
            self.sujo = true;
        }
    }

    pub fn avancar(&mut self, passo: isize) {
        let n = self.apps.len() as isize;
        if n == 0 {
            return;
        }
        self.escolhido = (self.escolhido as isize + passo).rem_euclid(n) as usize;
        self.sujo = true;
    }

    pub fn janela_escolhida(&self) -> Option<HWND> {
        self.apps.get(self.escolhido).map(|a| a.janela)
    }

    /// Clique num ícone: escolhe esse app.
    pub fn clique(&mut self, x: i32) -> Option<HWND> {
        let e = self.sup.dpi / 96.0;
        let i = ((x as f32 / e - FOLGA) / CELULA).floor();
        if i >= 0.0 && (i as usize) < self.apps.len() {
            self.escolhido = i as usize;
            return self.janela_escolhida();
        }
        None
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let e = self.sup.dpi / 96.0;
        let (l, _) = janela::tamanho_cliente(self.sup.hwnd);
        let cabem = (((l as f32 / e) - FOLGA * 2.0) / CELULA).floor().max(1.0) as usize;
        // Se não couberem todos, a janela rola para manter o escolhido visível.
        let inicio = self.escolhido.saturating_sub(cabem.saturating_sub(1));
        let nome: Vec<u16> = self.apps.get(self.escolhido).map(|a| a.nome.encode_utf16().collect()).unwrap_or_default();
        let apps = &self.apps;
        let escolhido = self.escolhido;
        let pincel = &self.pincel;
        let formato = &self.formato;
        self.sup.desenhar(gpu, |dc| unsafe {
            for (k, a) in apps.iter().enumerate().skip(inicio).take(cabem) {
                let cx = FOLGA + (k - inicio) as f32 * CELULA + CELULA / 2.0;
                let cy = 18.0 + ICONE / 2.0 + 6.0;
                if k == escolhido {
                    let r = retangulo(cx - CELULA / 2.0 + 4.0, 12.0, cx + CELULA / 2.0 - 4.0, cy + ICONE / 2.0 + 10.0);
                    pincel.SetColor(&crate::tema::tinta(0.16));
                    dc.FillRoundedRectangle(&arredondado(r, 16.0), pincel);
                }
                let r = retangulo(cx - ICONE / 2.0, cy - ICONE / 2.0, cx + ICONE / 2.0, cy + ICONE / 2.0);
                match &a.icone {
                    Some(b) => dc.DrawBitmap(b, Some(&r), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None),
                    None => {
                        pincel.SetColor(&crate::tema::tinta(0.2));
                        dc.FillRoundedRectangle(&arredondado(r, 18.0), pincel);
                    }
                }
            }
            pincel.SetColor(&crate::tema::tinta(0.92));
            dc.DrawText(
                &nome,
                formato,
                &retangulo(0.0, ALTURA - 34.0, l as f32 / e, ALTURA - 10.0),
                pincel,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        })?;
        self.sujo = false;
        Ok(())
    }
}
