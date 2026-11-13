//! Layouts de janelas: cada monitor ganha uma grade de áreas (2 colunas,
//! 2x2, 3x2...). Arrastando uma janela com Shift apertado, as áreas aparecem
//! na tela e, ao soltar, a janela se encaixa na área sob o mouse. "Organizar
//! janelas" distribui as janelas abertas do monitor pelas áreas de uma vez.
//!
//! Opcional (desligado por padrão). Parado não gasta nada: só durante um
//! arrasto de janela o Cais olha o mouse (a cada 16 ms).

use crate::encaixe;
use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

/// (nome na tela, chave no config.txt)
pub const LAYOUTS: [(&str, &str); 7] = [
    ("2 colunas", "colunas2"),
    ("3 colunas", "colunas3"),
    ("Grade 2x2 (4 janelas)", "grade2x2"),
    ("Grade 3x2 (6 janelas)", "grade3x2"),
    ("Grade 3x3 (9 janelas)", "grade3x3"),
    ("Uma grande e duas pequenas", "grande2"),
    ("Uma grande e quatro pequenas", "grande4"),
];

/// (nome, chave, espaço em DIPs)
pub const ESPACOS: [(&str, &str, f32); 3] = [("Nenhum", "nenhum", 0.0), ("Pequeno", "pequeno", 6.0), ("Médio", "medio", 14.0)];

/// Durante um arrasto: olhar o mouse e o Shift (na janela do dock).
pub const TIMER_LAYOUT: usize = 9;

static LIGADO: AtomicBool = AtomicBool::new(false);
static PRINCIPAL: AtomicUsize = AtomicUsize::new(2);
static OUTROS: AtomicUsize = AtomicUsize::new(0);
static ESPACO: AtomicUsize = AtomicUsize::new(1);

pub fn indice_layout(chave: &str) -> Option<usize> {
    LAYOUTS.iter().position(|l| l.1 == chave)
}

pub fn indice_espaco(chave: &str) -> Option<usize> {
    ESPACOS.iter().position(|e| e.1 == chave)
}

/// A App avisa o que está no config.
pub fn configurar(ligado: bool, principal: usize, outros: usize, espaco: usize) {
    LIGADO.store(ligado, Ordering::Relaxed);
    PRINCIPAL.store(principal.min(LAYOUTS.len() - 1), Ordering::Relaxed);
    OUTROS.store(outros.min(LAYOUTS.len() - 1), Ordering::Relaxed);
    ESPACO.store(espaco.min(ESPACOS.len() - 1), Ordering::Relaxed);
}

pub fn ligado() -> bool {
    LIGADO.load(Ordering::Relaxed)
}

/// Áreas em frações do monitor: (x0, y0, x1, y1).
fn fracoes(k: usize) -> Vec<[f32; 4]> {
    let grade = |c: usize, l: usize| -> Vec<[f32; 4]> {
        let mut v = Vec::new();
        for j in 0..l {
            for i in 0..c {
                v.push([i as f32 / c as f32, j as f32 / l as f32, (i + 1) as f32 / c as f32, (j + 1) as f32 / l as f32]);
            }
        }
        v
    };
    match LAYOUTS.get(k).map(|l| l.1).unwrap_or("grade2x2") {
        "colunas2" => grade(2, 1),
        "colunas3" => grade(3, 1),
        "grade3x2" => grade(3, 2),
        "grade3x3" => grade(3, 3),
        "grande2" => vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 0.5], [0.5, 0.5, 1.0, 1.0]],
        "grande4" => vec![
            [0.0, 0.0, 0.5, 1.0],
            [0.5, 0.0, 0.75, 0.5],
            [0.75, 0.0, 1.0, 0.5],
            [0.5, 0.5, 0.75, 1.0],
            [0.75, 0.5, 1.0, 1.0],
        ],
        _ => grade(2, 2),
    }
}

/// Layout escolhido para o monitor.
fn layout_do_monitor(mon: HMONITOR) -> usize {
    if mon == janela::hmonitor_principal() {
        PRINCIPAL.load(Ordering::Relaxed)
    } else {
        OUTROS.load(Ordering::Relaxed)
    }
}

/// Áreas do monitor em px da tela, dentro da área livre (sem a barra do
/// topo, o dock e a faixa da esquerda).
pub fn zonas(mon: HMONITOR) -> Vec<RECT> {
    let (_, w) = janela::info_monitor(mon);
    let dpi = unsafe {
        let mut x = 96u32;
        let mut y = 96u32;
        let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(mon, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut x, &mut y);
        x.max(96) as f32
    };
    let g = (ESPACOS[ESPACO.load(Ordering::Relaxed)].2 * dpi / 96.0).round() as i32;
    let (lw, aw) = ((w.right - w.left) as f32, (w.bottom - w.top) as f32);
    fracoes(layout_do_monitor(mon))
        .into_iter()
        .map(|[x0, y0, x1, y1]| {
            let borda = |f: f32, inicio: bool| -> i32 {
                if f <= 0.0 || f >= 1.0 {
                    if inicio { g } else { -g }
                } else if inicio {
                    g / 2
                } else {
                    -(g - g / 2)
                }
            };
            RECT {
                left: w.left + (x0 * lw).round() as i32 + borda(x0, true),
                top: w.top + (y0 * aw).round() as i32 + borda(y0, true),
                right: w.left + (x1 * lw).round() as i32 + borda(x1, false),
                bottom: w.top + (y1 * aw).round() as i32 + borda(y1, false),
            }
        })
        .collect()
}

/// Encaixa a janela na área (tirando do maximizado, se for o caso).
pub fn encaixar(h: HWND, area: RECT) {
    unsafe {
        if IsHungAppWindow(h).as_bool() {
            return;
        }
        if IsZoomed(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(h, SW_RESTORE);
        }
    }
    encaixe::posicionar_visivel(h, area);
}

/// Distribui as janelas abertas do monitor pelas áreas (a da frente na
/// primeira área). As que sobram ficam onde estão.
pub fn organizar(mon: HMONITOR) {
    let areas = zonas(mon);
    let janelas: Vec<HWND> = crate::apps::enumerar_janelas()
        .into_iter()
        .map(|(h, _)| h)
        .filter(|&h| unsafe {
            IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() && MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) == mon
        })
        .collect();
    for (h, a) in janelas.into_iter().zip(areas) {
        encaixar(h, a);
    }
}

/// Organiza todos os monitores.
pub fn organizar_todos() {
    for m in janela::monitores() {
        organizar(m);
    }
}

// ----------------------------------------------------------------------
// Arrastar com Shift

/// Janela transparente que mostra as áreas por cima de tudo (sem pegar
/// clique) enquanto o Shift está apertado num arrasto.
struct Painel {
    sup: Superficie,
    mon: HMONITOR,
    /// Origem do monitor (px) e as áreas.
    origem: (i32, i32),
    areas: Vec<RECT>,
    destaque: Option<usize>,
}

impl Drop for Painel {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

impl Painel {
    fn new(gpu: &Gpu, mon: HMONITOR) -> Result<Self> {
        let (m, _) = janela::info_monitor(mon);
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT)?;
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, m.left, m.top, m.right - m.left, m.bottom - m.top, SWP_NOACTIVATE);
        }
        let sup = Superficie::new(gpu, hwnd)?;
        Ok(Self { sup, mon, origem: (m.left, m.top), areas: zonas(mon), destaque: None })
    }

    fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let e = self.sup.dpi / 96.0;
        let (ox, oy) = self.origem;
        let areas = self.areas.clone();
        let destaque = self.destaque;
        self.sup.desenhar(gpu, |dc| unsafe {
            let Ok(p) = dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None) else { return };
            for (i, a) in areas.iter().enumerate() {
                let r = retangulo(
                    (a.left - ox) as f32 / e + 4.0,
                    (a.top - oy) as f32 / e + 4.0,
                    (a.right - ox) as f32 / e - 4.0,
                    (a.bottom - oy) as f32 / e - 4.0,
                );
                let rr = arredondado(r, 14.0);
                let sim = destaque == Some(i);
                p.SetColor(&if sim { crate::tema::destaque_a(0.38) } else { crate::tema::fundo(0.08, 0.08, 0.1, 0.28) });
                dc.FillRoundedRectangle(&rr, &p);
                p.SetColor(&if sim { crate::tema::destaque_vivo(0.95) } else { crate::tema::tinta(0.45) });
                dc.DrawRoundedRectangle(&rr, &p, if sim { 3.0 } else { 1.5 }, None);
            }
        })?;
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
    }
}

/// Um arrasto de janela em andamento.
pub struct Arrasto {
    janela: HWND,
    painel: Option<Painel>,
}

impl Arrasto {
    /// Começou a arrastar (ou redimensionar) uma janela.
    pub fn comecar(dono: HWND, h: HWND) -> Option<Self> {
        if !ligado() {
            return None;
        }
        unsafe {
            SetTimer(dono, TIMER_LAYOUT, 16, None);
        }
        Some(Self { janela: h, painel: None })
    }

    /// A cada 16 ms durante o arrasto.
    pub fn olhar(&mut self, gpu: &Gpu) {
        let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
        if !shift {
            self.painel = None;
            return;
        }
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let mon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
        if self.painel.as_ref().is_some_and(|p| p.mon != mon) {
            self.painel = None;
        }
        if self.painel.is_none() {
            self.painel = Painel::new(gpu, mon).ok();
            if let Some(p) = &mut self.painel {
                p.destaque = Some(usize::MAX); // força o 1º desenho
            }
        }
        let Some(p) = &mut self.painel else { return };
        let sob = p.areas.iter().position(|a| pt.x >= a.left && pt.x < a.right && pt.y >= a.top && pt.y < a.bottom);
        if sob != p.destaque {
            p.destaque = sob;
            let _ = p.desenhar(gpu);
        }
    }

    /// Soltou: encaixa na área sob o mouse (se o Shift estava apertado).
    pub fn terminar(self, dono: HWND) {
        unsafe {
            let _ = KillTimer(dono, TIMER_LAYOUT);
        }
        // O painel só existe com o Shift apertado no último instante.
        let area = self.painel.as_ref().and_then(|p| p.destaque.and_then(|i| p.areas.get(i).copied()));
        let h = self.janela;
        drop(self);
        match area {
            Some(a) => encaixar(h, a),
            _ => encaixe::ajustar(h),
        }
    }
}
