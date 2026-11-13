//! Busca no estilo Spotlight: um campo de vidro no alto da tela que acha
//! apps, páginas de Configurações e faz contas enquanto você digita.
//! Abre pelo atalho (Alt+Espaço por padrão) ou pela lupa da barra do topo.

use crate::apps;
use crate::campo::{self, Campo, Tecla};
use crate::catalogo::{Catalogo, Resultado, Tipo};
use crate::gpu::Gpu;
use crate::icones;
use crate::janela;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const LARGURA: f32 = 680.0;
const ALT_CAMPO: f32 = 58.0;
const ALT_LINHA: f32 = 46.0;
const MAX: usize = 8;
const RAIO: f32 = 14.0;

pub struct Busca {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    grande: IDWriteTextFormat,
    titulo: IDWriteTextFormat,
    detalhe: IDWriteTextFormat,
    campo: Campo,
    resultados: Vec<Resultado>,
    sel: usize,
    altura: f32,
    pub visivel: bool,
    /// Escolheram "Buscar arquivos": a App abre o Arquivos com este texto.
    pub pedido_arquivos: Option<String>,
    fechado_em: Option<Instant>,
    pub sujo: bool,
    /// Arquivos achados pelo índice do Windows para `consulta`.
    arquivos: Vec<std::path::PathBuf>,
    consulta: String,
    geracao: u32,
    icones_arquivos: std::collections::HashMap<String, Option<ID2D1Bitmap1>>,
}

/// Quantos arquivos do índice entram na lista.
const MAX_ARQUIVOS: u32 = 4;

impl Busca {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        janela::posicionar(hwnd, 0, 0, LARGURA as i32, ALT_CAMPO as i32);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, RAIO);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |familia, peso, tam, alinhar_direita: bool| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(
                    familia,
                    None,
                    peso,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    tam,
                    crate::idioma::localidade(),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                if alinhar_direita {
                    f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
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
            Ok(Self {
                sup,
                vidro,
                pincel,
                grande: fonte(w!("Segoe UI Variable Display"), DWRITE_FONT_WEIGHT_LIGHT, 22.0, false)?,
                titulo: fonte(w!("Segoe UI Variable Text"), DWRITE_FONT_WEIGHT_NORMAL, 15.0, false)?,
                detalhe: fonte(w!("Segoe UI Variable Text"), DWRITE_FONT_WEIGHT_NORMAL, 12.0, true)?,
                campo: Campo::default(),
                resultados: Vec::new(),
                sel: 0,
                altura: ALT_CAMPO,
                visivel: false,
                pedido_arquivos: None,
                fechado_em: None,
                sujo: false,
                arquivos: Vec::new(),
                consulta: String::new(),
                geracao: 0,
                icones_arquivos: std::collections::HashMap::new(),
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
        self.fechado_em.map_or(false, |t| t.elapsed().as_millis() < 300)
    }

    pub fn abrir(&mut self, gpu: &Gpu, cat: &mut Catalogo) {
        self.visivel = true;
        // Como no Mac: a última busca volta selecionada (digitar substitui).
        self.campo.tudo = !self.campo.texto.is_empty();
        self.campo.cursor = self.campo.texto.len();
        cat.atualizar();
        self.refazer(gpu, cat);
        let e = self.escala();
        let (mon, _) = janela::monitor_do_cursor();
        let l = (LARGURA * e).round() as i32;
        let x = mon.left + (mon.right - mon.left - l) / 2;
        let y = mon.top + ((mon.bottom - mon.top) as f32 * 0.2) as i32;
        janela::posicionar(self.sup.hwnd, x, y, l, (self.altura * e).round() as i32);
        let _ = self.ajustar_tamanho(gpu);
        let _ = self.desenhar(gpu, cat);
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.sup.hwnd);
        }
        self.sup.animar_entrada(gpu, 0.0, 0.97);
    }

    pub fn fechar(&mut self) {
        if !self.visivel {
            return;
        }
        self.visivel = false;
        self.fechado_em = Some(Instant::now());
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_HIDE);
        }
    }

    /// Recalcula os resultados (texto mudou ou a lista de apps chegou).
    pub fn refazer(&mut self, gpu: &Gpu, cat: &Catalogo) {
        let texto = self.campo.string();
        self.resultados = cat.buscar(&texto, MAX);
        // Arquivos (nome e conteúdo) pelo índice do Windows, sem travar a
        // digitação: a resposta chega depois e entra antes de "Buscar arquivos".
        let limpo = texto.trim().to_string();
        if limpo != self.consulta {
            self.consulta = limpo.clone();
            self.arquivos.clear();
            if limpo.chars().count() >= 3 && !self.resultados.iter().any(|r| r.tipo == Tipo::Conta) {
                self.geracao = crate::indice::buscar(limpo, MAX_ARQUIVOS, self.sup.hwnd);
            } else {
                crate::indice::cancelar();
            }
        }
        if !self.arquivos.is_empty() {
            let pos = self.resultados.iter().position(|r| r.tipo == Tipo::Arquivos).unwrap_or(self.resultados.len());
            let achados: Vec<Resultado> = self
                .arquivos
                .iter()
                .map(|p| Resultado {
                    titulo: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    detalhe: p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    tipo: Tipo::Arquivo,
                    app: None,
                    valor: p.to_string_lossy().into_owned(),
                })
                .collect();
            self.resultados.splice(pos..pos, achados);
        }
        self.sel = self.sel.min(self.resultados.len().saturating_sub(1));
        let antes = self.altura;
        self.altura = if self.resultados.is_empty() {
            ALT_CAMPO
        } else {
            ALT_CAMPO + 1.0 + 8.0 + self.resultados.len() as f32 * ALT_LINHA + 8.0
        };
        if self.visivel && (self.altura - antes).abs() > 0.5 {
            let _ = self.ajustar_tamanho(gpu);
        }
        self.sujo = true;
    }

    fn ajustar_tamanho(&mut self, gpu: &Gpu) -> Result<()> {
        let e = self.escala();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let (l, a) = ((LARGURA * e).round() as i32, (self.altura * e).round() as i32);
        janela::posicionar(self.sup.hwnd, rc.left, rc.top, l, a);
        self.sup.redimensionar(gpu)?;
        self.vidro.definir(0.0, 0.0, l as f32, a as f32, RAIO * e)?;
        self.sujo = true;
        Ok(())
    }

    fn linha_em(&self, y: f32) -> Option<usize> {
        let y0 = ALT_CAMPO + 1.0 + 8.0;
        if y < y0 {
            return None;
        }
        let i = ((y - y0) / ALT_LINHA) as usize;
        (i < self.resultados.len()).then_some(i)
    }

    fn executar(&mut self, cat: &mut Catalogo, i: usize) {
        let Some(r) = self.resultados.get(i).cloned() else { return };
        self.fechar();
        match r.tipo {
            Tipo::App => {
                if let Some(k) = r.app {
                    cat.abrir_app(k);
                }
            }
            Tipo::Ajuste => {
                apps::abrir(&r.valor);
                cat.usou(&r.valor);
            }
            Tipo::Conta => campo::copiar(&r.valor),
            Tipo::Arquivos => self.pedido_arquivos = Some(r.valor),
            Tipo::Arquivo => apps::abrir(&r.valor),
        }
    }

    pub fn mensagem(&mut self, gpu: &Gpu, cat: &mut Catalogo, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    self.fechar();
                }
                Some(LRESULT(0))
            }
            WM_CHAR => {
                if let Tecla::Mudou = self.campo.caractere(wp.0 as u32) {
                    self.sel = 0;
                    self.refazer(gpu, cat);
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                match vk {
                    // Um Esc fecha (o texto volta selecionado na próxima vez).
                    VK_ESCAPE => self.fechar(),
                    VK_RETURN => self.executar(cat, self.sel),
                    VK_DOWN => {
                        if self.sel + 1 < self.resultados.len() {
                            self.sel += 1;
                            self.sujo = true;
                        }
                    }
                    VK_UP => {
                        self.sel = self.sel.saturating_sub(1);
                        self.sujo = true;
                    }
                    _ => match self.campo.tecla(vk.0) {
                        Tecla::Mudou => {
                            self.sel = 0;
                            self.refazer(gpu, cat);
                        }
                        Tecla::Moveu => self.sujo = true,
                        Tecla::Nada => {}
                    },
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if let Some(i) = self.linha_em(y) {
                    if i != self.sel {
                        self.sel = i;
                        self.sujo = true;
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if let Some(i) = self.linha_em(y) {
                    self.executar(cat, i);
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let _ = self.ajustar_tamanho(gpu);
                Some(LRESULT(0))
            }
            crate::indice::WM_ARQUIVOS => {
                if let Some(v) = crate::indice::pegar(wp.0 as u32).filter(|_| wp.0 as u32 == self.geracao) {
                    self.arquivos = v;
                    self.refazer(gpu, cat);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu, cat: &mut Catalogo) -> Result<()> {
        let px = (30.0 * self.escala()).ceil() as u32;
        let mut icones_apps: Vec<Option<ID2D1Bitmap1>> = Vec::new();
        for r in &self.resultados {
            icones_apps.push(match r.tipo {
                Tipo::App => cat.icone(&gpu.d2d_dc, &r.valor),
                Tipo::Arquivo => self
                    .icones_arquivos
                    .entry(r.valor.clone())
                    .or_insert_with(|| apps::icone_sem_cache(&gpu.d2d_dc, &r.valor, px))
                    .clone(),
                _ => None,
            });
        }
        if self.icones_arquivos.len() > 64 {
            self.icones_arquivos.clear();
        }
        let texto: Vec<u16> = self.campo.string().encode_utf16().collect();
        let vazio = texto.is_empty();
        let mostrar: Vec<u16> = if vazio {
            crate::idioma::t("Buscar apps, arquivos, ajustes ou fazer contas", "Search apps, files, settings or do math")
                .encode_utf16()
                .collect()
        } else {
            texto.clone()
        };
        let x_texto = 54.0;
        let larg_texto = LARGURA - x_texto - 20.0;
        let campo_layout = unsafe { gpu.dwrite.CreateTextLayout(&mostrar, &self.grande, larg_texto, ALT_CAMPO)? };
        // Posição do cursor (em unidades UTF-16).
        let pos16: usize = self.campo.texto[..self.campo.cursor].iter().map(|c| c.len_utf16()).sum();
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            if !vazio {
                let _ = campo_layout.HitTestTextPosition(pos16 as u32, false, &mut cx, &mut cy, &mut m);
            }
        }
        let mut metr = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = campo_layout.GetMetrics(&mut metr);
        }
        let tudo = self.campo.tudo && !vazio;
        let mut linhas = Vec::new();
        for r in &self.resultados {
            let t: Vec<u16> = r.titulo.encode_utf16().collect();
            let d: Vec<u16> = r.detalhe.encode_utf16().collect();
            unsafe {
                linhas.push((
                    gpu.dwrite.CreateTextLayout(&t, &self.titulo, LARGURA - 64.0 - 170.0, ALT_LINHA)?,
                    gpu.dwrite.CreateTextLayout(&d, &self.detalhe, 150.0, ALT_LINHA)?,
                ));
            }
        }
        let p = &self.pincel;
        let resultados = &self.resultados;
        let sel = self.sel;
        self.sup.desenhar(gpu, |dc| unsafe {
            icones::lupa(dc, p, 27.0, ALT_CAMPO / 2.0, 1.25, crate::tema::tinta(0.6));
            if tudo {
                p.SetColor(&crate::tema::destaque_a(0.55));
                dc.FillRectangle(
                    &retangulo(x_texto - 1.0, ALT_CAMPO / 2.0 - 15.0, x_texto + metr.width + 1.0, ALT_CAMPO / 2.0 + 15.0),
                    p,
                );
            }
            p.SetColor(&crate::tema::tinta(if vazio { 0.38 } else { 0.95 }));
            dc.DrawTextLayout(D2D_POINT_2F { x: x_texto, y: 0.0 }, &campo_layout, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            if !tudo {
                // Cursor fixo (sem piscar: nada de timer acordando o Cais).
                let x = x_texto + if vazio { 0.0 } else { cx };
                p.SetColor(&crate::tema::destaque_vivo(1.0));
                dc.FillRectangle(&retangulo(x, ALT_CAMPO / 2.0 - 13.0, x + 2.0, ALT_CAMPO / 2.0 + 13.0), p);
            }
            if resultados.is_empty() {
                return;
            }
            p.SetColor(&crate::tema::tinta(0.12));
            dc.FillRectangle(&retangulo(0.0, ALT_CAMPO, LARGURA, ALT_CAMPO + 1.0), p);
            let mut y = ALT_CAMPO + 1.0 + 8.0;
            for (i, r) in resultados.iter().enumerate() {
                if i == sel {
                    p.SetColor(&crate::tema::destaque_a(0.92));
                    dc.FillRoundedRectangle(&arredondado(retangulo(8.0, y, LARGURA - 8.0, y + ALT_LINHA), 8.0), p);
                }
                let icx = 38.0;
                let icy = y + ALT_LINHA / 2.0;
                let branco = crate::tema::tinta_sobre(i == sel, 0.92);
                match (&r.tipo, &icones_apps[i]) {
                    (Tipo::App | Tipo::Arquivo, Some(b)) => dc.DrawBitmap(
                        b,
                        Some(&retangulo(icx - 15.0, icy - 15.0, icx + 15.0, icy + 15.0)),
                        1.0,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    ),
                    (Tipo::App | Tipo::Arquivo, None) => {
                        p.SetColor(&crate::tema::tinta(0.16));
                        dc.FillRoundedRectangle(
                            &arredondado(retangulo(icx - 14.0, icy - 14.0, icx + 14.0, icy + 14.0), 6.0),
                            p,
                        );
                    }
                    (Tipo::Ajuste, _) => icones::engrenagem(dc, p, icx, icy, 1.05, branco),
                    (Tipo::Conta, _) => icones::calculadora(dc, p, icx, icy, 1.0, branco),
                    (Tipo::Arquivos, _) => icones::lupa(dc, p, icx, icy, 1.0, branco),
                }
                let (lt, ld) = &linhas[i];
                p.SetColor(&crate::tema::tinta_sobre(i == sel, 0.96));
                dc.DrawTextLayout(D2D_POINT_2F { x: 64.0, y }, lt, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                p.SetColor(&crate::tema::tinta_sobre(i == sel, if i == sel { 0.8 } else { 0.5 }));
                dc.DrawTextLayout(D2D_POINT_2F { x: LARGURA - 24.0 - 150.0, y }, ld, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                y += ALT_LINHA;
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}
