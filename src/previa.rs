//! Balão que sai de um ícone do dock:
//! - Prévia: miniaturas ao vivo das janelas do app (parar o mouse no ícone).
//!   Clique numa para trazê-la.
//! - Pilha: os arquivos mais recentes de uma pasta do dock (Downloads etc.).
//!   Clique num arquivo para abrir, ou em "Abrir no Arquivos".

use crate::dock::Posicao;
use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::Vidro;
use std::path::{Path, PathBuf};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const FOLGA: f32 = 12.0;
const MINI_L: f32 = 220.0;
const MINI_A: f32 = 136.0;
const TITULO_A: f32 = 24.0;
const ARQ_L: f32 = 96.0;
const ARQ_A: f32 = 104.0;
const ARQ_ICONE: f32 = 52.0;
const CABECALHO: f32 = 34.0;
const RODAPE: f32 = 40.0;
const MAX_JANELAS: usize = 6;
const MAX_ARQUIVOS: usize = 16;

enum Conteudo {
    Janela { hwnd: HWND, miniatura: isize, titulo: Vec<u16> },
    Arquivo { caminho: PathBuf, icone: Option<ID2D1Bitmap1>, nome: Vec<u16> },
    AbrirPasta,
}

struct Item {
    /// Área clicável (DIPs).
    r: D2D_RECT_F,
    c: Conteudo,
}

pub struct Previa {
    sup: Superficie,
    _vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    texto: IDWriteTextFormat,
    pequeno: IDWriteTextFormat,
    itens: Vec<Item>,
    janelas: Vec<HWND>,
    cabecalho: Option<Vec<u16>>,
    pasta: Option<PathBuf>,
    hover: Option<usize>,
    pub fechar: bool,
    pub sujo: bool,
}

impl Drop for Previa {
    fn drop(&mut self) {
        for i in &self.itens {
            if let Conteudo::Janela { miniatura, .. } = i.c {
                unsafe {
                    let _ = DwmUnregisterThumbnail(miniatura);
                }
            }
        }
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

/// Onde o balão fica: ao lado do ícone, do lado de dentro da tela.
fn posicao_do_balao(ancora: RECT, posicao: Posicao, l: i32, a: i32) -> (i32, i32) {
    let (_, trab) = janela::monitor_principal();
    let (cx, cy) = ((ancora.left + ancora.right) / 2, (ancora.top + ancora.bottom) / 2);
    let (x, y) = match posicao {
        Posicao::Baixo => (cx - l / 2, ancora.top - 12 - a),
        Posicao::Esquerda => (ancora.right + 12, cy - a / 2),
        Posicao::Direita => (ancora.left - 12 - l, cy - a / 2),
    };
    (x.clamp(trab.left + 4, (trab.right - l - 4).max(trab.left + 4)), y.clamp(trab.top + 4, (trab.bottom - a - 4).max(trab.top + 4)))
}

fn largo(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Arquivos mais recentes de uma pasta (pela data de modificação).
pub fn recentes_da_pasta(pasta: &Path, max: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(pasta) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            let nome = e.file_name().to_string_lossy().to_lowercase();
            !nome.starts_with('.') && nome != "desktop.ini"
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(max).map(|(_, p)| p).collect()
}

impl Previa {
    fn base(gpu: &Gpu, l: f32, a: f32, ancora: RECT, posicao: Posicao) -> Result<(Superficie, Vidro, ID2D1SolidColorBrush, IDWriteTextFormat, IDWriteTextFormat)> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)?;
        janela::preparar_vidro(hwnd);
        let e = janela::escala(hwnd);
        let (lp, ap) = ((l * e).round() as i32, (a * e).round() as i32);
        let (x, y) = posicao_do_balao(ancora, posicao, lp, ap);
        janela::posicionar(hwnd, x, y, lp, ap);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 14.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, crate::tema::Tinta::Painel)?;
        vidro.definir(0.0, 0.0, lp as f32, ap as f32, 14.0 * e)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |tam: f32| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    tam,
                    crate::idioma::localidade(),
                )?;
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                let sinal = gpu.dwrite.CreateEllipsisTrimmingSign(&f)?;
                f.SetTrimming(
                    &DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 },
                    &sinal,
                )?;
                Ok(f)
            };
            let texto = fonte(12.5)?;
            texto.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let pequeno = fonte(11.5)?;
            Ok((sup, vidro, pincel, texto, pequeno))
        }
    }

    /// Miniaturas das janelas de um app.
    pub fn abrir(gpu: &Gpu, janelas: &[HWND], ancora: RECT, posicao: Posicao) -> Result<Self> {
        let n = janelas.len().min(MAX_JANELAS).max(1);
        let colunas = n.min(3);
        let linhas = n.div_ceil(colunas);
        let (l, a) = (FOLGA * 2.0 + colunas as f32 * MINI_L, FOLGA * 2.0 + linhas as f32 * (MINI_A + TITULO_A + 8.0));
        let (sup, vidro, pincel, texto, pequeno) = Self::base(gpu, l, a, ancora, posicao)?;
        let e = sup.dpi / 96.0;
        let mut itens = Vec::new();
        for (k, &h) in janelas.iter().take(MAX_JANELAS).enumerate() {
            let (col, lin) = (k % colunas, k / colunas);
            let x = FOLGA + col as f32 * MINI_L;
            let y = FOLGA + lin as f32 * (MINI_A + TITULO_A + 8.0);
            unsafe {
                let Ok(id) = DwmRegisterThumbnail(sup.hwnd, h) else { continue };
                let (sl, sa) = DwmQueryThumbnailSourceSize(id).map(|s| (s.cx.max(1) as f32, s.cy.max(1) as f32)).unwrap_or((16.0, 10.0));
                let s = ((MINI_L - 16.0) / sl).min((MINI_A - 8.0) / sa);
                let (ml, ma) = (sl * s, sa * s);
                let (mx, my) = (x + (MINI_L - ml) / 2.0, y + 4.0 + (MINI_A - 8.0 - ma) / 2.0);
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                    rcDestination: RECT {
                        left: (mx * e).round() as i32,
                        top: (my * e).round() as i32,
                        right: ((mx + ml) * e).round() as i32,
                        bottom: ((my + ma) * e).round() as i32,
                    },
                    rcSource: RECT::default(),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                let _ = DwmUpdateThumbnailProperties(id, &props);
                let mut buf = [0u16; 256];
                let nt = GetWindowTextW(h, &mut buf) as usize;
                itens.push(Item {
                    r: retangulo(x + 2.0, y, x + MINI_L - 2.0, y + MINI_A + TITULO_A + 4.0),
                    c: Conteudo::Janela { hwnd: h, miniatura: id, titulo: buf[..nt].to_vec() },
                });
            }
        }
        let p = Self { sup, _vidro: vidro, pincel, texto, pequeno, itens, janelas: janelas.to_vec(), cabecalho: None, pasta: None, hover: None, fechar: false, sujo: true };
        unsafe {
            let _ = ShowWindow(p.sup.hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(p)
    }

    /// Pilha: os arquivos recentes de uma pasta.
    pub fn abrir_pilha(gpu: &Gpu, pasta: &Path, ancora: RECT, posicao: Posicao) -> Result<Self> {
        let arquivos = recentes_da_pasta(pasta, MAX_ARQUIVOS);
        let colunas = 4usize;
        let linhas = arquivos.len().div_ceil(colunas).max(1);
        let (l, a) = (FOLGA * 2.0 + colunas as f32 * ARQ_L, CABECALHO + FOLGA + linhas as f32 * ARQ_A + RODAPE);
        let (sup, vidro, pincel, texto, pequeno) = Self::base(gpu, l, a, ancora, posicao)?;
        let e = sup.dpi / 96.0;
        let px = (ARQ_ICONE * e).ceil() as u32;
        let mut itens = Vec::new();
        for (k, c) in arquivos.into_iter().enumerate() {
            let (col, lin) = (k % colunas, k / colunas);
            let x = FOLGA + col as f32 * ARQ_L;
            let y = CABECALHO + lin as f32 * ARQ_A;
            let icone = crate::apps::icone_sem_cache(&gpu.d2d_dc, &c.to_string_lossy(), px);
            let nome = largo(&c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            itens.push(Item { r: retangulo(x + 2.0, y, x + ARQ_L - 2.0, y + ARQ_A - 4.0), c: Conteudo::Arquivo { caminho: c, icone, nome } });
        }
        itens.push(Item { r: retangulo(l / 2.0 - 90.0, a - RODAPE + 6.0, l / 2.0 + 90.0, a - 8.0), c: Conteudo::AbrirPasta });
        let titulo = pasta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let p = Self {
            sup,
            _vidro: vidro,
            pincel,
            texto,
            pequeno,
            itens,
            janelas: Vec::new(),
            cabecalho: Some(largo(&titulo)),
            pasta: Some(pasta.to_path_buf()),
            hover: None,
            fechar: false,
            sujo: true,
        };
        unsafe {
            let _ = ShowWindow(p.sup.hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(p)
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn janelas(&self) -> &[HWND] {
        &self.janelas
    }

    /// O ponto (tela, px) está dentro do balão?
    pub fn contem(&self, p: POINT) -> bool {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut r);
        }
        p.x >= r.left - 6 && p.x < r.right + 6 && p.y >= r.top - 6 && p.y < r.bottom + 6
    }

    pub fn mensagem(&mut self, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.sup.dpi / 96.0;
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        let em = |r: &D2D_RECT_F| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(3)), // MA_NOACTIVATE
            WM_MOUSEMOVE => {
                let h = self.itens.iter().position(|i| em(&i.r));
                if h != self.hover {
                    self.hover = h;
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if let Some(i) = self.itens.iter().position(|i| em(&i.r)) {
                    match &self.itens[i].c {
                        Conteudo::Janela { hwnd, .. } => crate::apps::trazer_janela(*hwnd),
                        Conteudo::Arquivo { caminho, .. } => crate::apps::abrir(&caminho.to_string_lossy()),
                        Conteudo::AbrirPasta => {
                            if let Some(p) = &self.pasta {
                                crate::apps::abrir(&p.to_string_lossy());
                            }
                        }
                    }
                    self.fechar = true;
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let (l, _) = janela::tamanho_cliente(self.sup.hwnd);
        let l = l as f32 / (self.sup.dpi / 96.0);
        let itens = &self.itens;
        let hover = self.hover;
        let pincel = &self.pincel;
        let texto = &self.texto;
        let pequeno = &self.pequeno;
        let cabecalho = &self.cabecalho;
        let abrir = largo(crate::idioma::t("Abrir no Arquivos", "Open in Files"));
        let vazio = largo(crate::idioma::t("Pasta vazia", "Empty folder"));
        let sem_arquivos = cabecalho.is_some() && itens.len() == 1;
        self.sup.desenhar(gpu, |dc| unsafe {
            if let Some(c) = cabecalho {
                pincel.SetColor(&crate::tema::tinta(0.95));
                dc.DrawText(c, texto, &retangulo(0.0, 9.0, l, CABECALHO), pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
            if sem_arquivos {
                pincel.SetColor(&crate::tema::tinta(0.6));
                dc.DrawText(&vazio, texto, &retangulo(0.0, CABECALHO + 30.0, l, CABECALHO + 60.0), pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
            for (k, i) in itens.iter().enumerate() {
                if hover == Some(k) {
                    pincel.SetColor(&crate::tema::tinta(0.12));
                    dc.FillRoundedRectangle(&arredondado(i.r, 10.0), pincel);
                }
                match &i.c {
                    Conteudo::Janela { titulo, .. } => {
                        pincel.SetColor(&crate::tema::tinta(0.9));
                        let r = retangulo(i.r.left + 6.0, i.r.bottom - TITULO_A - 2.0, i.r.right - 6.0, i.r.bottom);
                        dc.DrawText(titulo, texto, &r, pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    }
                    Conteudo::Arquivo { icone, nome, .. } => {
                        let cx = (i.r.left + i.r.right) / 2.0;
                        let ri = retangulo(cx - ARQ_ICONE / 2.0, i.r.top + 6.0, cx + ARQ_ICONE / 2.0, i.r.top + 6.0 + ARQ_ICONE);
                        match icone {
                            Some(b) => dc.DrawBitmap(b, Some(&ri), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None),
                            None => {
                                pincel.SetColor(&crate::tema::tinta(0.2));
                                dc.FillRoundedRectangle(&arredondado(ri, 8.0), pincel);
                            }
                        }
                        pincel.SetColor(&crate::tema::tinta(0.9));
                        let r = retangulo(i.r.left + 4.0, ri.bottom + 4.0, i.r.right - 4.0, i.r.bottom);
                        dc.DrawText(nome, pequeno, &r, pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    }
                    Conteudo::AbrirPasta => {
                        pincel.SetColor(&crate::tema::destaque_a(if hover == Some(k) { 0.95 } else { 0.8 }));
                        dc.FillRoundedRectangle(&arredondado(i.r, 8.0), pincel);
                        pincel.SetColor(&crate::tema::tinta_sobre(true, 1.0));
                        let r = retangulo(i.r.left, i.r.top + 6.0, i.r.right, i.r.bottom);
                        dc.DrawText(&abrir, texto, &r, pincel, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    }
                }
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}
