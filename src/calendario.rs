//! Calendário que abre ao clicar no relógio da barra do topo: o mês com o
//! dia de hoje marcado, setas para outros meses e um botão para a Central
//! de Notificações do Windows (o Cais não lê as notificações de ninguém).

use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

const LARGURA: f32 = 300.0;
const ALTURA: f32 = 360.0;
const P: f32 = 16.0;
const CEL: f32 = (LARGURA - 2.0 * P) / 7.0;
const MESES: [&str; 12] = [
    "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro",
];
const SEMANA: [&str; 7] = ["domingo", "segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado"];
const MESES_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];
const SEMANA_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

#[derive(Clone, Copy, PartialEq)]
enum Alvo {
    Anterior,
    Proximo,
    Hoje,
    Notificacoes,
}

pub struct Calendario {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    titulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    pequeno: IDWriteTextFormat,
    /// Mês mostrado (ano, mês 1-12).
    mes: (i32, u32),
    hover: Option<Alvo>,
    pub visivel: bool,
    fechado_em: Option<std::time::Instant>,
    /// Pediu a Central de Notificações do Windows.
    pub pedido_notificacoes: bool,
    pub sujo: bool,
}

/// Hoje: (ano, mês, dia, dia da semana 0 = domingo).
fn hoje() -> (i32, u32, u32, u32) {
    let t = unsafe { GetLocalTime() };
    (t.wYear as i32, t.wMonth as u32, t.wDay as u32, t.wDayOfWeek as u32)
}

fn bissexto(a: i32) -> bool {
    (a % 4 == 0 && a % 100 != 0) || a % 400 == 0
}

fn dias_no_mes(a: i32, m: u32) -> u32 {
    match m {
        2 if bissexto(a) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Dia da semana (0 = domingo) do dia d (método de Sakamoto).
fn dia_da_semana(a: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let a = if m < 3 { a - 1 } else { a };
    ((a + a / 4 - a / 100 + a / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7)) as u32
}

impl Calendario {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        janela::posicionar(hwnd, 0, 0, LARGURA as i32, ALTURA as i32);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 16.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |peso, tam, centro: bool| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(w!("Segoe UI Variable Text"), None, peso, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, tam, crate::idioma::localidade())?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                if centro {
                    f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                }
                Ok(f)
            };
            let (a, m, ..) = hoje();
            Ok(Self {
                sup,
                vidro,
                pincel,
                titulo: fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 15.0, false)?,
                texto: fonte(DWRITE_FONT_WEIGHT_NORMAL, 13.0, true)?,
                pequeno: fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 11.0, true)?,
                mes: (a, m),
                hover: None,
                visivel: false,
                fechado_em: None,
                pedido_notificacoes: false,
                sujo: false,
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    pub fn acabou_de_fechar(&self) -> bool {
        self.fechado_em.is_some_and(|t| t.elapsed().as_millis() < 300)
    }

    /// Abre com o canto superior direito em (direita, baixo) da tela.
    pub fn abrir(&mut self, gpu: &Gpu, direita: i32, baixo: i32) {
        let (a, m, ..) = hoje();
        self.mes = (a, m);
        self.visivel = true;
        let e = self.escala();
        let (l, h) = ((LARGURA * e).round() as i32, (ALTURA * e).round() as i32);
        janela::posicionar(self.sup.hwnd, direita - l - (8.0 * e) as i32, baixo + (6.0 * e) as i32, l, h);
        let _ = self.sup.redimensionar(gpu);
        let _ = self.vidro.definir(0.0, 0.0, l as f32, h as f32, 16.0 * e);
        let _ = self.desenhar(gpu);
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.sup.hwnd);
        }
        self.sup.animar_entrada(gpu, -8.0 * e, 1.0);
    }

    pub fn fechar(&mut self) {
        if !self.visivel {
            return;
        }
        self.visivel = false;
        self.fechado_em = Some(std::time::Instant::now());
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_HIDE);
        }
    }

    fn zonas(&self) -> [(D2D_RECT_F, Alvo); 4] {
        [
            (retangulo(LARGURA - P - 60.0, 12.0, LARGURA - P - 34.0, 40.0), Alvo::Anterior),
            (retangulo(LARGURA - P - 30.0, 12.0, LARGURA - P - 4.0, 40.0), Alvo::Proximo),
            (retangulo(P, 12.0, LARGURA - P - 66.0, 40.0), Alvo::Hoje),
            (retangulo(P, ALTURA - 14.0 - 34.0, LARGURA - P, ALTURA - 14.0), Alvo::Notificacoes),
        ]
    }

    fn acertar(&self, x: f32, y: f32) -> Option<Alvo> {
        self.zonas().into_iter().find(|(r, _)| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom).map(|z| z.1)
    }

    fn andar(&mut self, passo: i32) {
        let (a, m) = self.mes;
        let k = a * 12 + m as i32 - 1 + passo;
        self.mes = (k.div_euclid(12), (k.rem_euclid(12) + 1) as u32);
        self.sujo = true;
    }

    pub fn mensagem(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    self.fechar();
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                self.fechar();
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 == 0x25 => {
                self.andar(-1);
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 == 0x27 => {
                self.andar(1);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xffff) as i16;
                self.andar(if d > 0 { -1 } else { 1 });
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let h = self.acertar(x, y);
                if h != self.hover {
                    self.hover = h;
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                match self.acertar(x, y) {
                    Some(Alvo::Anterior) => self.andar(-1),
                    Some(Alvo::Proximo) => self.andar(1),
                    Some(Alvo::Hoje) => {
                        let (a, m, ..) = hoje();
                        self.mes = (a, m);
                        self.sujo = true;
                    }
                    Some(Alvo::Notificacoes) => {
                        self.fechar();
                        self.pedido_notificacoes = true;
                    }
                    None => {}
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let (a, m) = self.mes;
        let (ha, hm, hd, hs) = hoje();
        let dw = &gpu.dwrite;
        let lay = |s: &str, f: &IDWriteTextFormat, l: f32, h: f32| -> Result<IDWriteTextLayout> {
            let v: Vec<u16> = s.encode_utf16().collect();
            unsafe { dw.CreateTextLayout(&v, f, l, h) }
        };
        let en = crate::idioma::ingles();
        let titulo = if en {
            format!("{} {}", MESES_EN[(m - 1) as usize], a)
        } else {
            format!("{} de {}", MESES[(m - 1) as usize], a)
        };
        let titulo = lay(&titulo, &self.titulo, LARGURA - 2.0 * P - 66.0, 28.0)?;
        let iniciais = if en { ["S", "M", "T", "W", "T", "F", "S"] } else { ["D", "S", "T", "Q", "Q", "S", "S"] };
        let mut semana = Vec::new();
        for (k, s) in iniciais.iter().enumerate() {
            semana.push((lay(s, &self.pequeno, CEL, 20.0)?, P + k as f32 * CEL));
        }
        // Grade de 6 semanas: começa no domingo antes do dia 1.
        let primeiro = dia_da_semana(a, m, 1) as i32;
        let (ap, mp) = if m == 1 { (a - 1, 12) } else { (a, m - 1) };
        let dias_ant = dias_no_mes(ap, mp) as i32;
        let dias = dias_no_mes(a, m) as i32;
        let topo_grade = 78.0;
        let mut celulas = Vec::new(); // (layout, x, y, do mês, hoje)
        for c in 0..42 {
            let n = c - primeiro + 1;
            let (dia, do_mes) = if n < 1 { (dias_ant + n, false) } else if n > dias { (n - dias, false) } else { (n, true) };
            let x = P + (c % 7) as f32 * CEL;
            let y = topo_grade + (c / 7) as f32 * 34.0;
            let e_hoje = do_mes && a == ha && m == hm && dia as u32 == hd;
            celulas.push((lay(&dia.to_string(), &self.texto, CEL, 30.0)?, x, y, do_mes, e_hoje));
        }
        let data = if en {
            format!("{}, {} {}", SEMANA_EN[hs as usize], MESES_EN[(hm - 1) as usize], hd)
        } else {
            format!("{}, {} de {}", SEMANA[hs as usize], hd, MESES[(hm - 1) as usize])
        };
        let data = lay(&data, &self.texto, LARGURA - 2.0 * P, 22.0)?;
        let botao = lay(crate::idioma::t("Notificações do Windows", "Windows notifications"), &self.texto, LARGURA - 2.0 * P, 34.0)?;
        let zonas = self.zonas();
        let hover = self.hover;
        let p = &self.pincel;
        self.sup.desenhar(gpu, |dc| unsafe {
            p.SetColor(&crate::tema::tinta(0.95));
            dc.DrawTextLayout(D2D_POINT_2F { x: P, y: 12.0 }, &titulo, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            for (r, alvo) in &zonas[..2] {
                if hover == Some(*alvo) {
                    p.SetColor(&crate::tema::tinta(0.1));
                    dc.FillRoundedRectangle(&arredondado(*r, 6.0), p);
                }
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                let d = if *alvo == Alvo::Anterior { -1.0 } else { 1.0 };
                p.SetColor(&crate::tema::tinta(0.8));
                dc.DrawLine(D2D_POINT_2F { x: cx - 2.5 * d, y: cy - 5.0 }, D2D_POINT_2F { x: cx + 2.5 * d, y: cy }, p, 1.6, None);
                dc.DrawLine(D2D_POINT_2F { x: cx + 2.5 * d, y: cy }, D2D_POINT_2F { x: cx - 2.5 * d, y: cy + 5.0 }, p, 1.6, None);
            }
            p.SetColor(&crate::tema::tinta(0.45));
            for (l, x) in &semana {
                dc.DrawTextLayout(D2D_POINT_2F { x: *x, y: 52.0 }, l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
            for (l, x, y, do_mes, e_hoje) in &celulas {
                if *e_hoje {
                    p.SetColor(&crate::tema::destaque_a(1.0));
                    dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: x + CEL / 2.0, y: y + 15.0 }, radiusX: 14.0, radiusY: 14.0 }, p);
                }
                p.SetColor(&crate::tema::tinta_sobre(*e_hoje, if *do_mes { 0.92 } else { 0.3 }));
                dc.DrawTextLayout(D2D_POINT_2F { x: *x, y: *y }, l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
            p.SetColor(&crate::tema::tinta(0.08));
            dc.FillRectangle(&retangulo(P, ALTURA - 82.0, LARGURA - P, ALTURA - 81.0), p);
            p.SetColor(&crate::tema::tinta(0.6));
            dc.DrawTextLayout(D2D_POINT_2F { x: P, y: ALTURA - 76.0 }, &data, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            let b = zonas[3].0;
            p.SetColor(&crate::tema::tinta(if hover == Some(Alvo::Notificacoes) { 0.16 } else { 0.1 }));
            dc.FillRoundedRectangle(&arredondado(b, 8.0), p);
            p.SetColor(&crate::tema::tinta(0.92));
            dc.DrawTextLayout(D2D_POINT_2F { x: b.left, y: b.top }, &botao, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        })?;
        self.sujo = false;
        Ok(())
    }
}
