//! Launchpad: todos os apps instalados em tela cheia, sobre um vidro
//! desfocado, em páginas de 7 x 5. Roda do mouse ou setas trocam de
//! página; digitar filtra. A janela só existe enquanto está aberta.

use crate::campo::{Campo, Tecla};
use crate::catalogo::Catalogo;
use crate::gpu::Gpu;
use crate::icones;
use crate::janela;
use crate::mola::Mola;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::Vidro;
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const COLUNAS: usize = 7;
const LINHAS: usize = 5;
const POR_PAGINA: usize = COLUNAS * LINHAS;
const ICONE: f32 = 92.0;
const TOPO_GRADE: f32 = 120.0;
const BASE_GRADE: f32 = 120.0;
const CAMPO_L: f32 = 240.0;
const CAMPO_A: f32 = 30.0;
const CAMPO_Y: f32 = 46.0;
const TINTA: crate::tema::Tinta = crate::tema::Tinta::Launchpad;

pub struct Launchpad {
    sup: Superficie,
    _vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    rotulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    campo: Campo,
    filtrados: Vec<usize>,
    pagina: Mola,
    larg: f32,
    alt: f32,
    pressionado: Option<usize>,
    ultimo_giro: Instant,
    /// Pediu para fechar (a App solta a janela depois da mensagem).
    pub fechar: bool,
    pub sujo: bool,
}

impl Drop for Launchpad {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

impl Launchpad {
    pub fn abrir(gpu: &Gpu, cat: &mut Catalogo) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        let (mon, _) = janela::monitor_principal();
        janela::posicionar(hwnd, mon.left, mon.top, mon.right - mon.left, mon.bottom - mon.top);
        let sup = Superficie::new(gpu, hwnd)?;
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA)?;
        let (l, a) = janela::tamanho_cliente(hwnd);
        vidro.definir(0.0, 0.0, l as f32, a as f32, 0.0)?;
        let e = sup.dpi / 96.0;
        cat.atualizar();
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |tam, centro: bool| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    tam,
                    crate::idioma::localidade(),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                if centro {
                    f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                }
                let sinal = gpu.dwrite.CreateEllipsisTrimmingSign(&f)?;
                f.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    &sinal,
                )?;
                Ok(f)
            };
            let mut lp = Self {
                sup,
                _vidro: vidro,
                pincel,
                rotulo: fonte(13.0, true)?,
                texto: fonte(13.5, false)?,
                campo: Campo::default(),
                filtrados: Vec::new(),
                pagina: Mola::default(),
                larg: l as f32 / e,
                alt: a as f32 / e,
                pressionado: None,
                ultimo_giro: Instant::now(),
                fechar: false,
                sujo: true,
            };
            lp.refazer(cat);
            lp.desenhar(gpu, cat)?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            lp.sup.animar_entrada(gpu, 0.0, 1.06);
            Ok(lp)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    /// Lista de apps mudou (chegou da thread) ou o texto mudou.
    pub fn refazer(&mut self, cat: &Catalogo) {
        self.filtrados = cat.filtrar(&self.campo.string());
        if !self.campo.texto.is_empty() {
            self.pagina = Mola::default();
        }
        let ultima = self.paginas().saturating_sub(1) as f32;
        if self.pagina.alvo > ultima {
            self.pagina.alvo = ultima;
        }
        self.sujo = true;
    }

    pub fn sujar(&mut self) {
        self.sujo = true;
    }

    fn paginas(&self) -> usize {
        self.filtrados.len().div_ceil(POR_PAGINA).max(1)
    }

    pub fn animando(&self) -> bool {
        !self.pagina.parada()
    }

    pub fn passo(&mut self, dt: f32) {
        if self.animando() {
            self.pagina.passo(dt);
            self.sujo = true;
        }
    }

    fn ir_para(&mut self, p: isize) {
        let p = p.clamp(0, self.paginas() as isize - 1) as f32;
        if p != self.pagina.alvo {
            self.pagina.alvo = p;
            self.sujo = true;
        }
    }

    /// Geometria da grade: (x da primeira coluna, largura e altura da célula).
    /// Em telas grandes a grade não estica: fica centralizada.
    fn grade(&self) -> (f32, f32, f32) {
        let larg_grade = (self.larg * 0.8).min(COLUNAS as f32 * 190.0);
        let cel_l = larg_grade / COLUNAS as f32;
        let cel_a = ((self.alt - TOPO_GRADE - BASE_GRADE) / LINHAS as f32).min(175.0);
        ((self.larg - larg_grade) / 2.0, cel_l, cel_a)
    }

    /// Topo da grade, centralizada na vertical entre o campo e os pontinhos.
    fn topo_grade(&self) -> f32 {
        let (_, _, ca) = self.grade();
        let livre = self.alt - TOPO_GRADE - BASE_GRADE - ca * LINHAS as f32;
        TOPO_GRADE + livre / 2.0
    }

    /// Centro do ícone da posição k (dentro da página p).
    fn centro(&self, p: usize, k: usize) -> (f32, f32) {
        let (x0, cl, ca) = self.grade();
        let col = (k % COLUNAS) as f32;
        let lin = (k / COLUNAS) as f32;
        let desloc = (p as f32 - self.pagina.x) * self.larg;
        (x0 + cl * (col + 0.5) + desloc, self.topo_grade() + ca * lin + ca * 0.42)
    }

    /// Qual app (índice em `filtrados`) está em (x, y) na página atual.
    fn app_em(&self, x: f32, y: f32) -> Option<usize> {
        let p = self.pagina.alvo.round() as usize;
        let ini = p * POR_PAGINA;
        (ini..(ini + POR_PAGINA).min(self.filtrados.len())).find(|&i| {
            let (cx, cy) = self.centro(p, i - ini);
            (x - cx).abs() <= ICONE / 2.0 + 6.0 && y >= cy - ICONE / 2.0 - 6.0 && y <= cy + ICONE / 2.0 + 26.0
        })
    }

    fn ponto_em(&self, x: f32, y: f32) -> Option<usize> {
        let n = self.paginas();
        if n < 2 || (y - (self.alt - 70.0)).abs() > 10.0 {
            return None;
        }
        let x0 = self.larg / 2.0 - (n as f32 - 1.0) * 9.0;
        (0..n).find(|&i| (x - (x0 + i as f32 * 18.0)).abs() <= 8.0)
    }

    fn abrir_app(&mut self, cat: &mut Catalogo, i: usize) {
        if let Some(&k) = self.filtrados.get(i) {
            cat.abrir_app(k);
        }
        self.fechar = true;
    }

    pub fn mensagem(&mut self, cat: &mut Catalogo, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    self.fechar = true;
                }
                Some(LRESULT(0))
            }
            WM_CHAR => {
                if let Tecla::Mudou = self.campo.caractere(wp.0 as u32) {
                    self.refazer(cat);
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let vazio = self.campo.texto.is_empty();
                let atual = self.pagina.alvo.round() as isize;
                match vk {
                    VK_ESCAPE if vazio => self.fechar = true,
                    VK_ESCAPE => {
                        self.campo.limpar();
                        self.refazer(cat);
                    }
                    VK_RETURN if !self.filtrados.is_empty() && !vazio => self.abrir_app(cat, 0),
                    VK_LEFT | VK_PRIOR if vazio => self.ir_para(atual - 1),
                    VK_RIGHT | VK_NEXT if vazio => self.ir_para(atual + 1),
                    _ => match self.campo.tecla(vk.0) {
                        Tecla::Mudou => self.refazer(cat),
                        Tecla::Moveu => self.sujo = true,
                        Tecla::Nada => {}
                    },
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16;
                // Uma página por gesto (touchpads mandam vários avisos seguidos).
                if delta != 0 && self.ultimo_giro.elapsed().as_millis() > 280 {
                    self.ultimo_giro = Instant::now();
                    let frente = if msg == WM_MOUSEWHEEL { delta < 0 } else { delta > 0 };
                    let atual = self.pagina.alvo.round() as isize;
                    self.ir_para(if frente { atual + 1 } else { atual - 1 });
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                self.pressionado = self.app_em(x, y);
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let alvo = self.app_em(x, y);
                let antes = self.pressionado.take();
                self.sujo = true;
                if let Some(i) = alvo.filter(|_| alvo == antes) {
                    self.abrir_app(cat, i);
                } else if let Some(p) = self.ponto_em(x, y) {
                    self.ir_para(p as isize);
                } else if alvo.is_none() && !self.no_campo(x, y) {
                    self.fechar = true; // clique no vazio fecha
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn no_campo(&self, x: f32, y: f32) -> bool {
        (x - self.larg / 2.0).abs() <= CAMPO_L / 2.0 && (y - CAMPO_Y).abs() <= CAMPO_A / 2.0
    }

    pub fn desenhar(&mut self, gpu: &Gpu, cat: &mut Catalogo) -> Result<()> {
        // Só as páginas que aparecem (a atual e a vizinha durante a troca).
        let p0 = self.pagina.x.floor().max(0.0) as usize;
        let p1 = (self.pagina.x.ceil() as usize).min(self.paginas() - 1);
        let mut itens = Vec::new();
        for p in p0..=p1 {
            let ini = p * POR_PAGINA;
            for i in ini..(ini + POR_PAGINA).min(self.filtrados.len()) {
                let (cx, cy) = self.centro(p, i - ini);
                if cx < -ICONE || cx > self.larg + ICONE {
                    continue;
                }
                let app = &cat.apps[self.filtrados[i]];
                let nome: Vec<u16> = app.nome.encode_utf16().collect();
                let abrir = app.abrir.clone();
                let (_, cl, _) = self.grade();
                let layout = unsafe { gpu.dwrite.CreateTextLayout(&nome, &self.rotulo, cl - 10.0, 20.0)? };
                itens.push((cx, cy, cat.icone(&gpu.d2d_dc, &abrir), layout, i));
            }
        }
        let texto: Vec<u16> = if self.campo.texto.is_empty() {
            crate::idioma::t("Buscar", "Search").encode_utf16().collect()
        } else {
            self.campo.string().encode_utf16().collect()
        };
        let vazio = self.campo.texto.is_empty();
        let campo_layout = unsafe { gpu.dwrite.CreateTextLayout(&texto, &self.texto, CAMPO_L - 40.0, CAMPO_A)? };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = campo_layout.GetMetrics(&mut m);
        }
        let carregando = cat.carregando();
        let msg_vazio = if carregando {
            Some(crate::idioma::t("Carregando apps…", "Loading apps…"))
        } else if self.filtrados.is_empty() {
            Some(crate::idioma::t("Nenhum app encontrado", "No apps found"))
        } else {
            None
        };
        let aviso = match msg_vazio {
            Some(s) => {
                let w: Vec<u16> = s.encode_utf16().collect();
                Some(unsafe { gpu.dwrite.CreateTextLayout(&w, &self.rotulo, 400.0, 30.0)? })
            }
            None => None,
        };
        let p = &self.pincel;
        let (larg, alt) = (self.larg, self.alt);
        let paginas = self.paginas();
        let atual = self.pagina.x;
        let pressionado = self.pressionado;
        self.sup.desenhar(gpu, |dc| unsafe {
            // Campo de busca.
            let c = retangulo(larg / 2.0 - CAMPO_L / 2.0, CAMPO_Y - CAMPO_A / 2.0, larg / 2.0 + CAMPO_L / 2.0, CAMPO_Y + CAMPO_A / 2.0);
            p.SetColor(&crate::tema::tinta(0.13));
            dc.FillRoundedRectangle(&arredondado(c, 8.0), p);
            p.SetColor(&crate::tema::tinta(0.16));
            dc.DrawRoundedRectangle(&arredondado(c, 8.0), p, 1.0, None);
            // Vazio: lupa + "Buscar" centralizados. Com texto: à esquerda.
            let x_txt = if vazio { larg / 2.0 - m.width / 2.0 + 9.0 } else { c.left + 30.0 };
            icones::lupa(dc, p, x_txt - 14.0, CAMPO_Y, 0.85, crate::tema::tinta(0.6));
            p.SetColor(&crate::tema::tinta(if vazio { 0.55 } else { 0.95 }));
            dc.DrawTextLayout(D2D_POINT_2F { x: x_txt, y: CAMPO_Y - CAMPO_A / 2.0 }, &campo_layout, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            if !vazio {
                p.SetColor(&crate::tema::destaque_vivo(1.0));
                let xc = x_txt + m.widthIncludingTrailingWhitespace + 1.0;
                dc.FillRectangle(&retangulo(xc, CAMPO_Y - 9.0, xc + 1.6, CAMPO_Y + 9.0), p);
            }

            for (cx, cy, icone, layout, i) in &itens {
                let r = retangulo(cx - ICONE / 2.0, cy - ICONE / 2.0, cx + ICONE / 2.0, cy + ICONE / 2.0);
                let op = if pressionado == Some(*i) { 0.55 } else { 1.0 };
                match icone {
                    Some(b) => dc.DrawBitmap(b, Some(&r), op, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None),
                    None => {
                        p.SetColor(&crate::tema::tinta(0.12 * op));
                        dc.FillRoundedRectangle(&arredondado(r, ICONE * 0.22), p);
                    }
                }
                p.SetColor(&crate::tema::tinta(0.95 * op));
                let cl = layout.GetMaxWidth();
                dc.DrawTextLayout(
                    D2D_POINT_2F { x: cx - cl / 2.0, y: cy + ICONE / 2.0 + 6.0 },
                    layout,
                    p,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }

            if let Some(l) = &aviso {
                p.SetColor(&crate::tema::tinta(0.6));
                dc.DrawTextLayout(D2D_POINT_2F { x: larg / 2.0 - 200.0, y: alt / 2.0 - 15.0 }, l, p, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }

            // Pontinhos das páginas.
            if paginas > 1 {
                let x0 = larg / 2.0 - (paginas as f32 - 1.0) * 9.0;
                for k in 0..paginas {
                    let perto = 1.0 - (k as f32 - atual).abs().min(1.0);
                    p.SetColor(&crate::tema::tinta(0.32 + 0.63 * perto));
                    dc.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: x0 + k as f32 * 18.0, y: alt - 70.0 },
                            radiusX: 3.6,
                            radiusY: 3.6,
                        },
                        p,
                    );
                }
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}
