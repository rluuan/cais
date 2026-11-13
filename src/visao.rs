//! Visão Geral: todas as janelas abertas lado a lado, uma tela de vidro por
//! monitor. As miniaturas são as do próprio Windows (ao vivo, desenhadas
//! pelo compositor) e saem do lugar real de cada janela até a grade.
//! Clique numa janela para trazê-la; Esc ou clique no vazio fecha.

use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::Vidro;
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::*;

const ABRIR_MS: f32 = 260.0;
const FECHAR_MS: f32 = 200.0;
/// Folgas da grade (DIPs): em cima, dos lados, embaixo (o dock fica ali).
const TOPO: f32 = 70.0;
const LADO: f32 = 60.0;
const BASE: f32 = 110.0;
const ENTRE: f32 = 36.0;

struct Item {
    hwnd: HWND,
    miniatura: isize,
    /// Onde a janela está de verdade e onde fica na grade (px da janela).
    de: RECT,
    para: RECT,
    titulo: Vec<u16>,
}

struct Tela {
    sup: Superficie,
    _vidro: Vidro,
    itens: Vec<Item>,
    hover: Option<usize>,
}

pub struct Visao {
    telas: Vec<Tela>,
    pincel: ID2D1SolidColorBrush,
    formato: IDWriteTextFormat,
    vazio: IDWriteTextFormat,
    inicio: Instant,
    /// Fechando: a janela escolhida (se houver) é trazida no fim.
    fechando: Option<Option<HWND>>,
    /// Terminou de fechar (a App solta a Visão Geral).
    pub acabou: bool,
    pub sujo: bool,
    reduzir: bool,
}

impl Drop for Visao {
    fn drop(&mut self) {
        for t in &mut self.telas {
            for i in t.itens.drain(..) {
                unsafe {
                    let _ = DwmUnregisterThumbnail(i.miniatura);
                }
            }
            unsafe {
                let _ = DestroyWindow(t.sup.hwnd);
            }
        }
    }
}

/// Grade que aproveita melhor a área: testa de 1 a n colunas e fica com a
/// que deixa as janelas maiores (nunca maiores que o tamanho real).
/// `tams` = (largura, altura) de cada janela; devolve (x, y, l, a).
pub fn dispor(tams: &[(f32, f32)], area: (f32, f32, f32, f32), entre: f32) -> Vec<(f32, f32, f32, f32)> {
    let n = tams.len();
    if n == 0 {
        return Vec::new();
    }
    let (ax, ay, al, aa) = area;
    let mut melhor: Option<(f32, usize)> = None;
    for colunas in 1..=n {
        let linhas = n.div_ceil(colunas);
        let cl = (al - entre * (colunas - 1) as f32) / colunas as f32;
        let ca = (aa - entre * (linhas - 1) as f32) / linhas as f32;
        if cl <= 10.0 || ca <= 10.0 {
            continue;
        }
        let total: f32 = tams
            .iter()
            .map(|&(l, a)| {
                let s = (cl / l).min(ca / a).min(1.0);
                l * a * s * s
            })
            .sum();
        if melhor.map_or(true, |(m, _)| total > m) {
            melhor = Some((total, colunas));
        }
    }
    let colunas = melhor.map_or(1, |m| m.1);
    let linhas = n.div_ceil(colunas);
    let cl = (al - entre * (colunas - 1) as f32) / colunas as f32;
    let ca = (aa - entre * (linhas - 1) as f32) / linhas as f32;
    let mut v = Vec::with_capacity(n);
    for (k, &(l, a)) in tams.iter().enumerate() {
        let linha = k / colunas;
        let col = k % colunas;
        // A última linha, se incompleta, fica centralizada.
        let nesta = if linha == linhas - 1 { n - linha * colunas } else { colunas };
        let sobra = (colunas - nesta) as f32 * (cl + entre) / 2.0;
        let s = (cl / l).min(ca / a).min(1.0);
        let (w, h) = (l * s, a * s);
        let cx = ax + sobra + col as f32 * (cl + entre) + cl / 2.0;
        let cy = ay + linha as f32 * (ca + entre) + ca / 2.0;
        v.push((cx - w / 2.0, cy - h / 2.0, w, h));
    }
    v
}

fn suave(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn mistura(a: RECT, b: RECT, t: f32) -> RECT {
    let m = |x: i32, y: i32| (x as f32 + (y - x) as f32 * t).round() as i32;
    RECT { left: m(a.left, b.left), top: m(a.top, b.top), right: m(a.right, b.right), bottom: m(a.bottom, b.bottom) }
}

impl Visao {
    /// Abre em todos os monitores. None se não há janela nenhuma para mostrar.
    pub fn abrir(gpu: &Gpu, reduzir: bool) -> Result<Option<Self>> {
        // Janelas de app visíveis (não minimizadas), da frente para trás.
        let janelas: Vec<HWND> = crate::apps::enumerar_janelas()
            .into_iter()
            .map(|(h, _)| h)
            .filter(|h| unsafe { !IsIconic(*h).as_bool() })
            .collect();
        let mut telas = Vec::new();
        for m in janela::monitores() {
            let minhas: Vec<HWND> = janelas
                .iter()
                .copied()
                .filter(|h| unsafe { MonitorFromWindow(*h, MONITOR_DEFAULTTONEAREST) } == m)
                .collect();
            telas.push(Self::tela(gpu, m, &minhas)?);
        }
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let formato = gpu.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0,
                crate::idioma::localidade(),
            )?;
            formato.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let vazio = gpu.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                16.0,
                crate::idioma::localidade(),
            )?;
            vazio.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            vazio.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let mut v = Self {
                telas,
                pincel,
                formato,
                vazio,
                inicio: Instant::now(),
                fechando: None,
                acabou: false,
                sujo: true,
                reduzir,
            };
            v.passo();
            for t in &v.telas {
                let _ = ShowWindow(t.sup.hwnd, SW_SHOW);
            }
            if let Some(t) = v.telas.first() {
                let _ = SetForegroundWindow(t.sup.hwnd);
            }
            Ok(Some(v))
        }
    }

    fn tela(gpu: &Gpu, m: HMONITOR, janelas: &[HWND]) -> Result<Tela> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        let (mon, _) = janela::info_monitor(m);
        janela::posicionar(hwnd, mon.left, mon.top, mon.right - mon.left, mon.bottom - mon.top);
        let sup = Superficie::new(gpu, hwnd)?;
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, crate::tema::Tinta::Launchpad)?;
        let (l, a) = janela::tamanho_cliente(hwnd);
        vidro.definir(0.0, 0.0, l as f32, a as f32, 0.0)?;
        let e = sup.dpi / 96.0;
        let mut itens = Vec::new();
        for &h in janelas {
            let mut r = RECT::default();
            unsafe {
                if GetWindowRect(h, &mut r).is_err() {
                    continue;
                }
                let Ok(id) = DwmRegisterThumbnail(hwnd, h) else { continue };
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(h, &mut buf) as usize;
                itens.push(Item {
                    hwnd: h,
                    miniatura: id,
                    de: RECT { left: r.left - mon.left, top: r.top - mon.top, right: r.right - mon.left, bottom: r.bottom - mon.top },
                    para: RECT::default(),
                    titulo: buf[..n].to_vec(),
                });
            }
        }
        let tams: Vec<(f32, f32)> =
            itens.iter().map(|i| (((i.de.right - i.de.left).max(1)) as f32, ((i.de.bottom - i.de.top).max(1)) as f32)).collect();
        let area = (LADO * e, TOPO * e, l as f32 - 2.0 * LADO * e, a as f32 - (TOPO + BASE) * e);
        for (i, (x, y, w, h)) in itens.iter_mut().zip(dispor(&tams, area, ENTRE * e)) {
            i.para = RECT { left: x.round() as i32, top: y.round() as i32, right: (x + w).round() as i32, bottom: (y + h).round() as i32 };
        }
        Ok(Tela { sup, _vidro: vidro, itens, hover: None })
    }

    pub fn hwnds(&self) -> Vec<HWND> {
        self.telas.iter().map(|t| t.sup.hwnd).collect()
    }

    /// Progresso da animação (0 = janelas no lugar real, 1 = na grade).
    fn progresso(&self) -> f32 {
        let ms = self.inicio.elapsed().as_secs_f32() * 1000.0;
        if self.reduzir {
            return if self.fechando.is_some() { 0.0 } else { 1.0 };
        }
        match self.fechando {
            None => suave(ms / ABRIR_MS),
            Some(_) => 1.0 - suave(ms / FECHAR_MS),
        }
    }

    pub fn animando(&self) -> bool {
        if self.reduzir {
            return self.fechando.is_some() && !self.acabou;
        }
        let ms = self.inicio.elapsed().as_secs_f32() * 1000.0;
        !self.acabou && ms < if self.fechando.is_some() { FECHAR_MS } else { ABRIR_MS } + 20.0
    }

    /// Move as miniaturas para o quadro atual. No fim do fechamento, traz a
    /// janela escolhida e marca para a App soltar.
    pub fn passo(&mut self) {
        let p = self.progresso();
        for t in &self.telas {
            for (k, i) in t.itens.iter().enumerate() {
                let mut r = mistura(i.de, i.para, p);
                if t.hover == Some(k) && self.fechando.is_none() {
                    // A escolhida cresce um pouco.
                    let (dx, dy) = (((r.right - r.left) as f32 * 0.02) as i32, ((r.bottom - r.top) as f32 * 0.02) as i32);
                    r = RECT { left: r.left - dx, top: r.top - dy, right: r.right + dx, bottom: r.bottom + dy };
                }
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                    rcDestination: r,
                    rcSource: RECT::default(),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                unsafe {
                    let _ = DwmUpdateThumbnailProperties(i.miniatura, &props);
                }
            }
        }
        if let Some(escolhida) = self.fechando {
            if !self.animando() {
                self.acabou = true;
                if let Some(h) = escolhida {
                    crate::apps::trazer_janela(h);
                }
            }
        }
        self.sujo = true;
    }

    /// Começa a fechar (trazendo `h`, se houver).
    pub fn fechar(&mut self, h: Option<HWND>) {
        if self.fechando.is_some() {
            return;
        }
        self.fechando = Some(h);
        self.inicio = Instant::now();
        for t in &mut self.telas {
            t.hover = None;
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let mostrar_texto = self.progresso() > 0.95 && self.fechando.is_none();
        for t in &mut self.telas {
            let e = t.sup.dpi / 96.0;
            let pincel = &self.pincel;
            // Título da janela sob o mouse, logo abaixo da miniatura.
            let rotulo = match t.hover.filter(|_| mostrar_texto) {
                Some(k) => {
                    let i = &t.itens[k];
                    let largura = ((i.para.right - i.para.left) as f32 / e).max(120.0);
                    let layout = unsafe { gpu.dwrite.CreateTextLayout(&i.titulo, &self.formato, largura + 200.0, 30.0)? };
                    Some((layout, i.para))
                }
                None => None,
            };
            let vazio = t.itens.is_empty();
            let (l, a) = janela::tamanho_cliente(t.sup.hwnd);
            let (l, a) = (l as f32 / e, a as f32 / e);
            let aviso: Vec<u16> = crate::idioma::t("Nenhuma janela aberta neste monitor", "No open windows on this display")
                .encode_utf16()
                .collect();
            let fmt_vazio = &self.vazio;
            let destaque = t.hover.filter(|_| mostrar_texto).map(|k| t.itens[k].para);
            t.sup.desenhar(gpu, |dc| unsafe {
                if let Some(r) = destaque {
                    let rr = retangulo(r.left as f32 / e - 6.0, r.top as f32 / e - 6.0, r.right as f32 / e + 6.0, r.bottom as f32 / e + 6.0);
                    pincel.SetColor(&crate::tema::destaque_vivo(0.95));
                    dc.DrawRoundedRectangle(&arredondado(rr, 10.0), pincel, 3.0, None);
                }
                if let Some((layout, r)) = &rotulo {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = layout.GetMetrics(&mut m);
                    let cx = (r.left + r.right) as f32 / 2.0 / e;
                    let y = r.bottom as f32 / e + 14.0;
                    let caixa = retangulo(cx - m.width / 2.0 - 10.0, y, cx + m.width / 2.0 + 10.0, y + m.height + 8.0);
                    pincel.SetColor(&crate::tema::fundo(0.1, 0.1, 0.12, 0.85));
                    dc.FillRoundedRectangle(&arredondado(caixa, 7.0), pincel);
                    pincel.SetColor(&crate::tema::tinta(0.95));
                    dc.DrawTextLayout(D2D_POINT_2F { x: caixa.left + 10.0, y: y + 4.0 }, layout, pincel, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
                if vazio {
                    pincel.SetColor(&crate::tema::tinta(0.7));
                    dc.DrawText(&aviso, fmt_vazio, &retangulo(0.0, 0.0, l, a), pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                }
            })?;
        }
        self.sujo = false;
        Ok(())
    }

    fn item_em(t: &Tela, x: i32, y: i32) -> Option<usize> {
        t.itens.iter().position(|i| x >= i.para.left && x < i.para.right && y >= i.para.top && y < i.para.bottom)
    }

    /// Mensagens das janelas da Visão Geral.
    pub fn mensagem(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let Some(k) = self.telas.iter().position(|t| t.sup.hwnd == hwnd) else { return None };
        let x = (lp.0 & 0xffff) as i16 as i32;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
        match msg {
            WM_MOUSEMOVE => {
                let novo = Self::item_em(&self.telas[k], x, y);
                for (j, t) in self.telas.iter_mut().enumerate() {
                    let h = if j == k { novo } else { None };
                    if t.hover != h {
                        t.hover = h;
                        self.sujo = true;
                    }
                }
                if self.sujo {
                    self.passo();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let escolhida = Self::item_em(&self.telas[k], x, y).map(|i| self.telas[k].itens[i].hwnd);
                self.fechar(escolhida);
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.fechar(None);
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                match windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(wp.0 as u16) {
                    windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE => self.fechar(None),
                    windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN => {
                        let h = self.telas.iter().find_map(|t| t.hover.map(|i| t.itens[i].hwnd));
                        self.fechar(h);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE if wp.0 & 0xffff == 0 => {
                // Outra janela veio para frente (Alt+Tab, Win...): fecha.
                let outra = HWND(lp.0 as *mut _);
                if !self.telas.iter().any(|t| t.sup.hwnd == outra) {
                    self.fechar(None);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn grade_cabe_na_area() {
        let tams = vec![(1600.0, 900.0); 5];
        let area = (0.0, 0.0, 1920.0, 1000.0);
        let r = dispor(&tams, area, 20.0);
        assert_eq!(r.len(), 5);
        for &(x, y, w, h) in &r {
            assert!(x >= -0.01 && y >= -0.01, "{x},{y}");
            assert!(x + w <= 1920.01 && y + h <= 1000.01, "{x},{y},{w},{h}");
            // Mantém a proporção.
            assert!((w / h - 16.0 / 9.0).abs() < 0.01);
        }
        // Não se sobrepõem.
        for i in 0..r.len() {
            for j in i + 1..r.len() {
                let (a, b) = (r[i], r[j]);
                let separadas = a.0 + a.2 <= b.0 + 0.01 || b.0 + b.2 <= a.0 + 0.01 || a.1 + a.3 <= b.1 + 0.01 || b.1 + b.3 <= a.1 + 0.01;
                assert!(separadas, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn janela_pequena_nao_cresce() {
        let r = dispor(&[(300.0, 200.0)], (0.0, 0.0, 1920.0, 1000.0), 20.0);
        assert_eq!((r[0].2, r[0].3), (300.0, 200.0));
    }

    #[test]
    fn sem_janelas() {
        assert!(dispor(&[], (0.0, 0.0, 100.0, 100.0), 10.0).is_empty());
    }
}
