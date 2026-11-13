//! Central de Controle: painel de vidro que desce da barra do topo com
//! Wi-Fi, Bluetooth, brilho, som (geral, saída e por app) e a mídia tocando.
//! Só existe enquanto está aberto: fechado, não desenha nem consulta nada.

use crate::apps;
use crate::gpu::Gpu;
use crate::icones;
use crate::janela;
use crate::sistema::{Cmd, Estado, Rede};
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const LARGURA: f32 = 320.0;
const M: f32 = 10.0; // margem externa e espaço entre blocos
const P: f32 = 12.0; // espaço interno dos blocos
const RAIO: f32 = 16.0;
const LINHA: f32 = 48.0;
const MAX_APPS: usize = 8;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Acao {
    Wifi,
    AbrirWifi,
    Bluetooth,
    AbrirBluetooth,
    AbrirRede,
    Brilho,
    Volume,
    Mudo,
    Saidas,
    Saida(usize),
    AbrirSom,
    Anterior,
    TocarPausar,
    Proxima,
    VolumeApp(usize),
    MudoApp(usize),
}

impl Acao {
    fn deslizante(self) -> bool {
        matches!(self, Acao::Brilho | Acao::Volume | Acao::VolumeApp(_))
    }
}

#[derive(Clone, Copy)]
enum Fonte {
    Titulo,
    Texto,
    Pequeno,
}

#[derive(Clone, Copy)]
enum Icone {
    Wifi,
    Rede,
    Bluetooth,
}

enum Elem {
    Bloco(D2D_RECT_F),
    Botao { r: D2D_RECT_F, ligado: bool, icone: Icone, acao: Acao },
    Texto { r: D2D_RECT_F, s: String, fonte: Fonte, alfa: f32 },
    Linha { r: D2D_RECT_F, acao: Acao },
    Deslizador { r: D2D_RECT_F, valor: f32, acao: Acao, mudo: bool },
    Marca { x: f32, y: f32 },
    Seta { x: f32, y: f32, aberta: bool },
    IconeApp { r: D2D_RECT_F, exe: String, mudo: bool, acao: Acao },
    Midia { r: D2D_RECT_F, acao: Acao, tocando: bool },
}

pub struct Central {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    titulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    pequeno: IDWriteTextFormat,
    tx: Sender<Cmd>,
    estado: Estado,
    elems: Vec<Elem>,
    altura: f32,
    hover: Option<Acao>,
    arrasto: Option<(Acao, f32)>,
    saidas_abertas: bool,
    icones: HashMap<String, Option<ID2D1Bitmap1>>,
    pub visivel: bool,
    fechado_em: Option<Instant>,
    rastreando: bool,
    pub sujo: bool,
}

impl Central {
    pub fn new(gpu: &Gpu, tx: Sender<Cmd>) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        janela::posicionar(hwnd, 0, 0, 320, 200);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, RAIO);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |peso, tam| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    peso,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    tam,
                    crate::idioma::localidade(),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                // Nomes longos terminam em "…" em vez de vazar do bloco.
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
                titulo: fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0)?,
                texto: fonte(DWRITE_FONT_WEIGHT_NORMAL, 13.0)?,
                pequeno: fonte(DWRITE_FONT_WEIGHT_NORMAL, 11.5)?,
                tx,
                estado: Estado::default(),
                elems: Vec::new(),
                altura: 0.0,
                hover: None,
                arrasto: None,
                saidas_abertas: false,
                icones: HashMap::new(),
                visivel: false,
                fechado_em: None,
                rastreando: false,
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

    fn enviar(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    /// Fechou agora há pouco (o mesmo clique que fechou não deve reabrir).
    pub fn acabou_de_fechar(&self) -> bool {
        self.fechado_em.map_or(false, |t| t.elapsed().as_millis() < 300)
    }

    pub fn definir(&mut self, gpu: &Gpu, e: &Estado) {
        if *e != self.estado {
            self.estado = e.clone();
            if self.visivel {
                self.relayout(gpu);
            }
        }
    }

    /// Abre logo abaixo da barra do topo, encostado à direita.
    pub fn abrir(&mut self, gpu: &Gpu, direita: i32, topo: i32) {
        self.visivel = true;
        self.saidas_abertas = false;
        self.hover = None;
        self.arrasto = None;
        self.enviar(Cmd::Painel(true));
        self.montar();
        let e = self.escala();
        let l = (LARGURA * e).round() as i32;
        let a = (self.altura * e).round() as i32;
        let x = direita - l - (8.0 * e) as i32;
        let y = topo + (6.0 * e) as i32;
        janela::posicionar(self.sup.hwnd, x, y, l, a);
        let _ = self.ajustar_tamanho(gpu);
        let _ = self.desenhar(gpu);
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.sup.hwnd);
        }
        self.sup.animar_entrada(gpu, -8.0 * self.escala(), 1.0);
    }

    pub fn fechar(&mut self) {
        if !self.visivel {
            return;
        }
        self.visivel = false;
        self.fechado_em = Some(Instant::now());
        self.arrasto = None;
        unsafe {
            let _ = ReleaseCapture();
            let _ = ShowWindow(self.sup.hwnd, SW_HIDE);
        }
        self.enviar(Cmd::Painel(false));
        self.sujo = false;
    }

    fn relayout(&mut self, gpu: &Gpu) {
        let antes = self.altura;
        self.montar();
        if (self.altura - antes).abs() > 0.5 {
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
        let l = (LARGURA * e).round() as i32;
        let a = (self.altura * e).round() as i32;
        janela::posicionar(self.sup.hwnd, rc.left, rc.top, l, a);
        self.sup.redimensionar(gpu)?;
        self.vidro.definir(0.0, 0.0, l as f32, a as f32, RAIO * e)?;
        self.sujo = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Montagem (posições de tudo, em DIPs)

    fn montar(&mut self) {
        let e = &self.estado;
        let mut v: Vec<Elem> = Vec::new();
        let x0 = M;
        let x1 = LARGURA - M;
        let mut y = M;

        // Conexões: Wi-Fi (ou rede com cabo) e Bluetooth.
        let mut linhas: Vec<(Icone, bool, Acao, Acao, &str, String)> = Vec::new();
        match e.wifi {
            Some(ligado) => {
                let sub = if !ligado {
                    crate::idioma::t("Desligado", "Off").to_string()
                } else {
                    match &e.rede {
                        Rede::Wifi { nome, .. } => nome.clone(),
                        Rede::Cabo { .. } => crate::idioma::t("Ligado", "On").to_string(),
                        Rede::Nenhuma => crate::idioma::t("Sem conexão", "Not connected").to_string(),
                    }
                };
                linhas.push((Icone::Wifi, ligado, Acao::Wifi, Acao::AbrirWifi, "Wi-Fi", sub));
            }
            None => {
                let (ligado, sub) = match &e.rede {
                    Rede::Cabo { nome } if !nome.is_empty() => (true, nome.clone()),
                    Rede::Cabo { .. } => (true, crate::idioma::t("Conectado", "Connected").to_string()),
                    Rede::Wifi { nome, .. } => (true, nome.clone()),
                    Rede::Nenhuma => (false, crate::idioma::t("Sem conexão", "Not connected").to_string()),
                };
                linhas.push((Icone::Rede, ligado, Acao::AbrirRede, Acao::AbrirRede, crate::idioma::t("Rede", "Network"), sub));
            }
        }
        if let Some(ligado) = e.bluetooth {
            let sub = if ligado { crate::idioma::t("Ligado", "On") } else { crate::idioma::t("Desligado", "Off") }.to_string();
            linhas.push((
                Icone::Bluetooth,
                ligado,
                Acao::Bluetooth,
                Acao::AbrirBluetooth,
                "Bluetooth",
                sub,
            ));
        }
        let alt = linhas.len() as f32 * LINHA + 8.0;
        v.push(Elem::Bloco(retangulo(x0, y, x1, y + alt)));
        let mut ly = y + 4.0;
        for (icone, ligado, botao, abrir, titulo, sub) in linhas {
            let cy = ly + LINHA / 2.0;
            v.push(Elem::Linha {
                r: retangulo(x0 + 4.0, ly + 2.0, x1 - 4.0, ly + LINHA - 2.0),
                acao: abrir,
            });
            v.push(Elem::Botao {
                r: retangulo(x0 + P, cy - 15.0, x0 + P + 30.0, cy + 15.0),
                ligado,
                icone,
                acao: botao,
            });
            let tx = x0 + P + 42.0;
            v.push(Elem::Texto {
                r: retangulo(tx, cy - 17.0, x1 - 30.0, cy),
                s: titulo.to_string(),
                fonte: Fonte::Titulo,
                alfa: 0.95,
            });
            v.push(Elem::Texto {
                r: retangulo(tx, cy, x1 - 30.0, cy + 15.0),
                s: sub,
                fonte: Fonte::Pequeno,
                alfa: 0.6,
            });
            v.push(Elem::Seta { x: x1 - P - 2.0, y: cy, aberta: false });
            ly += LINHA;
        }
        y += alt + M;

        // Tela (brilho), se o monitor deixar.
        if let Some(b) = e.brilho {
            v.push(Elem::Bloco(retangulo(x0, y, x1, y + 66.0)));
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 8.0, x1 - P, y + 26.0),
                s: crate::idioma::t("Tela", "Display").into(),
                fonte: Fonte::Titulo,
                alfa: 0.95,
            });
            v.push(Elem::Deslizador {
                r: retangulo(x0 + P, y + 32.0, x1 - P, y + 56.0),
                valor: b as f32 / 100.0,
                acao: Acao::Brilho,
                mudo: false,
            });
            y += 66.0 + M;
        }

        // Som: volume geral, saída e lista de saídas.
        if e.tem_som || !e.saidas.is_empty() {
            let nome_saida = e
                .saidas
                .iter()
                .find(|s| s.id == e.saida_atual)
                .map(|s| s.nome.clone())
                .unwrap_or_else(|| crate::idioma::t("Nenhuma saída", "No output").into());
            let n = if self.saidas_abertas { e.saidas.len() + 1 } else { 0 };
            let alt = 64.0 + 32.0 + n as f32 * 28.0 + 6.0;
            v.push(Elem::Bloco(retangulo(x0, y, x1, y + alt)));
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 8.0, x1 - P, y + 26.0),
                s: crate::idioma::t("Som", "Sound").into(),
                fonte: Fonte::Titulo,
                alfa: 0.95,
            });
            v.push(Elem::Deslizador {
                r: retangulo(x0 + P, y + 32.0, x1 - P, y + 56.0),
                valor: e.volume,
                acao: Acao::Volume,
                mudo: e.mudo,
            });
            let mut ly = y + 62.0;
            v.push(Elem::Linha {
                r: retangulo(x0 + 4.0, ly, x1 - 4.0, ly + 30.0),
                acao: Acao::Saidas,
            });
            v.push(Elem::Texto {
                r: retangulo(x0 + P, ly, x1 - 30.0, ly + 30.0),
                s: format!("{}: {nome_saida}", crate::idioma::t("Saída", "Output")),
                fonte: Fonte::Texto,
                alfa: 0.8,
            });
            v.push(Elem::Seta { x: x1 - P - 2.0, y: ly + 15.0, aberta: self.saidas_abertas });
            ly += 32.0;
            if self.saidas_abertas {
                for (i, s) in e.saidas.iter().enumerate() {
                    v.push(Elem::Linha {
                        r: retangulo(x0 + 4.0, ly, x1 - 4.0, ly + 28.0),
                        acao: Acao::Saida(i),
                    });
                    if s.id == e.saida_atual {
                        v.push(Elem::Marca { x: x0 + P + 6.0, y: ly + 14.0 });
                    }
                    v.push(Elem::Texto {
                        r: retangulo(x0 + P + 20.0, ly, x1 - P, ly + 28.0),
                        s: s.nome.clone(),
                        fonte: Fonte::Texto,
                        alfa: 0.92,
                    });
                    ly += 28.0;
                }
                v.push(Elem::Linha {
                    r: retangulo(x0 + 4.0, ly, x1 - 4.0, ly + 28.0),
                    acao: Acao::AbrirSom,
                });
                v.push(Elem::Texto {
                    r: retangulo(x0 + P + 20.0, ly, x1 - P, ly + 28.0),
                    s: crate::idioma::t("Configurações de som…", "Sound settings…").into(),
                    fonte: Fonte::Texto,
                    alfa: 0.7,
                });
            }
            y += alt + M;
        }

        // Tocando agora.
        if let Some(m) = &e.midia {
            v.push(Elem::Bloco(retangulo(x0, y, x1, y + 72.0)));
            let tx1 = x1 - P - 100.0;
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 8.0, tx1, y + 22.0),
                s: m.app.clone(),
                fonte: Fonte::Pequeno,
                alfa: 0.55,
            });
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 23.0, tx1, y + 43.0),
                s: m.titulo.clone(),
                fonte: Fonte::Titulo,
                alfa: 0.95,
            });
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 43.0, tx1, y + 61.0),
                s: m.artista.clone(),
                fonte: Fonte::Texto,
                alfa: 0.65,
            });
            let cy = y + 36.0;
            for (k, acao) in [Acao::Anterior, Acao::TocarPausar, Acao::Proxima].into_iter().enumerate() {
                let cx = x1 - P - 84.0 + k as f32 * 34.0 + 16.0;
                v.push(Elem::Midia {
                    r: retangulo(cx - 16.0, cy - 16.0, cx + 16.0, cy + 16.0),
                    acao,
                    tocando: m.tocando,
                });
            }
            y += 72.0 + M;
        }

        // Volume de cada app que está usando o som.
        if !e.apps.is_empty() {
            let n = e.apps.len().min(MAX_APPS);
            let alt = 34.0 + n as f32 * 34.0 + 4.0;
            v.push(Elem::Bloco(retangulo(x0, y, x1, y + alt)));
            v.push(Elem::Texto {
                r: retangulo(x0 + P, y + 8.0, x1 - P, y + 26.0),
                s: crate::idioma::t("Volume dos apps", "App volume").into(),
                fonte: Fonte::Titulo,
                alfa: 0.95,
            });
            let mut ly = y + 34.0;
            for (i, a) in e.apps.iter().take(n).enumerate() {
                let cy = ly + 17.0;
                v.push(Elem::IconeApp {
                    r: retangulo(x0 + P, cy - 11.0, x0 + P + 22.0, cy + 11.0),
                    exe: a.exe.clone(),
                    mudo: a.mudo,
                    acao: Acao::MudoApp(i),
                });
                v.push(Elem::Texto {
                    r: retangulo(x0 + P + 30.0, cy - 10.0, x0 + P + 122.0, cy + 10.0),
                    s: a.nome.clone(),
                    fonte: Fonte::Texto,
                    alfa: 0.9,
                });
                v.push(Elem::Deslizador {
                    r: retangulo(x0 + P + 130.0, cy - 9.0, x1 - P, cy + 9.0),
                    valor: a.volume,
                    acao: Acao::VolumeApp(i),
                    mudo: a.mudo,
                });
                ly += 34.0;
            }
            y += alt + M;
        }

        self.elems = v;
        self.altura = y;
    }

    // ------------------------------------------------------------------
    // Mouse e teclado

    fn zona(&self, x: f32, y: f32) -> Option<(Acao, D2D_RECT_F)> {
        let dentro = |r: &D2D_RECT_F| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
        // De trás para frente: o que foi montado por último fica por cima.
        for el in self.elems.iter().rev() {
            let (r, acao) = match el {
                Elem::Botao { r, acao, .. }
                | Elem::Linha { r, acao }
                | Elem::IconeApp { r, acao, .. }
                | Elem::Midia { r, acao, .. } => (r, *acao),
                Elem::Deslizador { r, acao, .. } => {
                    // O alto-falante dentro da barra de volume liga o mudo.
                    if *acao == Acao::Volume && dentro(r) && x < r.left + 28.0 {
                        return Some((Acao::Mudo, *r));
                    }
                    (r, *acao)
                }
                _ => continue,
            };
            if dentro(r) {
                return Some((acao, *r));
            }
        }
        None
    }

    fn valor_em(r: &D2D_RECT_F, x: f32) -> f32 {
        ((x - r.left) / (r.right - r.left)).clamp(0.0, 1.0)
    }

    fn deslizar(&mut self, acao: Acao, v: f32) {
        self.arrasto = Some((acao, v));
        match acao {
            Acao::Brilho => self.enviar(Cmd::DefinirBrilho((v * 100.0).round() as u32)),
            Acao::Volume => self.enviar(Cmd::Volume(v)),
            Acao::VolumeApp(i) => {
                if let Some(a) = self.estado.apps.get(i) {
                    self.enviar(Cmd::VolumeApp(a.exe.clone(), v));
                }
            }
            _ => {}
        }
        self.sujo = true;
    }

    /// Mensagens da janela do painel. Devolve Some se tratou.
    pub fn mensagem(&mut self, gpu: &Gpu, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
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
            WM_MOUSEMOVE => {
                if !self.rastreando {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.sup.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    self.rastreando = true;
                }
                if let Some((acao, _)) = self.arrasto {
                    if let Some(r) = self.retangulo_de(acao) {
                        self.deslizar(acao, Self::valor_em(&r, x));
                    }
                } else {
                    let novo = self.zona(x, y).map(|z| z.0);
                    if novo != self.hover {
                        self.hover = novo;
                        self.sujo = true;
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.rastreando = false;
                if self.hover.take().is_some() {
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                if let Some((acao, r)) = self.zona(x, y) {
                    if acao.deslizante() {
                        unsafe {
                            SetCapture(self.sup.hwnd);
                        }
                        self.deslizar(acao, Self::valor_em(&r, x));
                    } else {
                        self.clicar(gpu, acao);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if self.arrasto.take().is_some() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                self.arrasto = None;
                None
            }
            WM_DPICHANGED => {
                let _ = self.ajustar_tamanho(gpu);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn retangulo_de(&self, acao: Acao) -> Option<D2D_RECT_F> {
        self.elems.iter().find_map(|el| match el {
            Elem::Deslizador { r, acao: a, .. } if *a == acao => Some(*r),
            _ => None,
        })
    }

    fn clicar(&mut self, gpu: &Gpu, acao: Acao) {
        let e = &mut self.estado;
        match acao {
            Acao::Wifi => {
                let novo = !e.wifi.unwrap_or(false);
                e.wifi = Some(novo);
                let _ = self.tx.send(Cmd::LigarWifi(novo));
            }
            Acao::Bluetooth => {
                let novo = !e.bluetooth.unwrap_or(false);
                e.bluetooth = Some(novo);
                let _ = self.tx.send(Cmd::LigarBluetooth(novo));
            }
            Acao::Mudo => {
                e.mudo = !e.mudo;
                let _ = self.tx.send(Cmd::Mudo(e.mudo));
            }
            Acao::MudoApp(i) => {
                if let Some(a) = e.apps.get_mut(i) {
                    a.mudo = !a.mudo;
                    let _ = self.tx.send(Cmd::MudoApp(a.exe.clone(), a.mudo));
                }
            }
            Acao::Saidas => {
                self.saidas_abertas = !self.saidas_abertas;
            }
            Acao::Saida(i) => {
                if let Some(s) = e.saidas.get(i) {
                    e.saida_atual = s.id.clone();
                    let _ = self.tx.send(Cmd::UsarSaida(s.id.clone()));
                }
                self.saidas_abertas = false;
            }
            Acao::TocarPausar => {
                if let Some(m) = &mut e.midia {
                    m.tocando = !m.tocando;
                }
                let _ = self.tx.send(Cmd::TocarPausar);
            }
            Acao::Anterior => {
                let _ = self.tx.send(Cmd::Anterior);
            }
            Acao::Proxima => {
                let _ = self.tx.send(Cmd::Proxima);
            }
            Acao::AbrirWifi | Acao::AbrirBluetooth | Acao::AbrirRede | Acao::AbrirSom => {
                self.fechar();
                apps::abrir(match acao {
                    Acao::AbrirWifi => "ms-availablenetworks:",
                    Acao::AbrirBluetooth => "ms-settings:bluetooth",
                    Acao::AbrirRede => "ms-settings:network",
                    _ => "ms-settings:sound",
                });
                return;
            }
            Acao::Brilho | Acao::Volume | Acao::VolumeApp(_) => {}
        }
        self.relayout(gpu);
    }

    // ------------------------------------------------------------------
    // Desenho

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        // Ícones dos apps (lidos do disco/cache uma vez só).
        for el in &self.elems {
            if let Elem::IconeApp { exe, .. } = el {
                if !exe.is_empty() && !self.icones.contains_key(exe) {
                    let b = apps::carregar_icone(&gpu.d2d_dc, exe, (22.0 * self.escala()).round() as u32);
                    self.icones.insert(exe.clone(), b);
                }
            }
        }
        let hover = self.hover;
        let arrasto = self.arrasto;
        let elems = &self.elems;
        let p = &self.pincel;
        let icones = &self.icones;
        let fontes = [&self.titulo, &self.texto, &self.pequeno];
        let dwrite = &gpu.dwrite;
        self.sup.desenhar(gpu, |dc| unsafe {
            for el in elems {
                match el {
                    Elem::Bloco(r) => {
                        p.SetColor(&crate::tema::tinta(0.07));
                        dc.FillRoundedRectangle(&arredondado(*r, 12.0), p);
                        p.SetColor(&crate::tema::tinta(0.09));
                        dc.DrawRoundedRectangle(&arredondado(*r, 12.0), p, 1.0, None);
                    }
                    Elem::Linha { r, acao } => {
                        if hover == Some(*acao) {
                            p.SetColor(&crate::tema::tinta(0.08));
                            dc.FillRoundedRectangle(&arredondado(*r, 8.0), p);
                        }
                    }
                    Elem::Botao { r, ligado, icone, acao } => {
                        let cx = (r.left + r.right) / 2.0;
                        let cy = (r.top + r.bottom) / 2.0;
                        let realce = if hover == Some(*acao) { 0.08 } else { 0.0 };
                        p.SetColor(&if *ligado {
                            { let (r, g, b) = crate::tema::destaque(); cor(r + realce, g + realce, b, 1.0) }
                        } else {
                            crate::tema::tinta(0.16 + realce)
                        });
                        dc.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: cx, y: cy },
                                radiusX: 15.0,
                                radiusY: 15.0,
                            },
                            p,
                        );
                        let branco = crate::tema::tinta_sobre(*ligado, 0.95);
                        match icone {
                            Icone::Wifi => icones::wifi(dc, p, cx, cy - 1.0, 0.85, 3, (branco.r, branco.g, branco.b)),
                            Icone::Rede => icones::cabo(dc, p, cx, cy, 0.85, branco),
                            Icone::Bluetooth => icones::bluetooth(dc, p, cx, cy, 0.95, branco),
                        }
                    }
                    Elem::Texto { r, s, fonte, alfa } => {
                        let f = fontes[*fonte as usize];
                        let w: Vec<u16> = s.encode_utf16().collect();
                        if let Ok(l) = dwrite.CreateTextLayout(&w, f, r.right - r.left, r.bottom - r.top) {
                            p.SetColor(&crate::tema::tinta(*alfa));
                            dc.DrawTextLayout(
                                D2D_POINT_2F { x: r.left, y: r.top },
                                &l,
                                p,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                            );
                        }
                    }
                    Elem::Deslizador { r, valor, acao, mudo } => {
                        let v = match arrasto {
                            Some((a, v)) if a == *acao => v,
                            _ => *valor,
                        };
                        let h = r.bottom - r.top;
                        let raio = h / 2.0;
                        p.SetColor(&crate::tema::tinta(0.14));
                        dc.FillRoundedRectangle(&arredondado(*r, raio), p);
                        let cheio = (r.left + h + (r.right - r.left - h) * v).min(r.right);
                        p.SetColor(&if crate::tema::escuro() { crate::tema::tinta(if *mudo { 0.4 } else { 0.92 }) } else { crate::tema::branco(if *mudo { 0.6 } else { 0.98 }) });
                        dc.FillRoundedRectangle(&arredondado(retangulo(r.left, r.top, cheio, r.bottom), raio), p);
                        if h >= 20.0 {
                            let cx = r.left + 14.0;
                            let cy = (r.top + r.bottom) / 2.0;
                            let escuro = if crate::tema::escuro() { cor(0.16, 0.16, 0.18, 0.9) } else { crate::tema::tinta(0.7) };
                            match acao {
                                Acao::Brilho => icones::sol(dc, p, cx, cy, 0.95, escuro),
                                _ => icones::som(dc, p, cx + 2.0, cy, 0.8, v, *mudo, escuro),
                            }
                        }
                    }
                    Elem::Marca { x, y } => {
                        p.SetColor(&crate::tema::tinta(0.95));
                        icones::marca(dc, p, *x, *y);
                    }
                    Elem::Seta { x, y, aberta } => {
                        p.SetColor(&crate::tema::tinta(0.45));
                        icones::seta(dc, p, *x, *y, *aberta);
                    }
                    Elem::IconeApp { r, exe, mudo, acao } => {
                        if hover == Some(*acao) {
                            p.SetColor(&crate::tema::tinta(0.12));
                            dc.FillRoundedRectangle(
                                &arredondado(
                                    retangulo(r.left - 3.0, r.top - 3.0, r.right + 3.0, r.bottom + 3.0),
                                    6.0,
                                ),
                                p,
                            );
                        }
                        let opac = if *mudo { 0.35 } else { 1.0 };
                        match icones.get(exe).and_then(|b| b.as_ref()) {
                            Some(b) => dc.DrawBitmap(
                                b,
                                Some(r),
                                opac,
                                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                                None,
                                None,
                            ),
                            None => {
                                let cx = (r.left + r.right) / 2.0;
                                let cy = (r.top + r.bottom) / 2.0;
                                icones::som(dc, p, cx + 2.0, cy, 0.85, 1.0, false, crate::tema::tinta(0.85 * opac));
                            }
                        }
                        if *mudo {
                            p.SetColor(&cor(1.0, 0.36, 0.33, 1.0));
                            dc.DrawLine(
                                D2D_POINT_2F { x: r.left, y: r.bottom },
                                D2D_POINT_2F { x: r.right, y: r.top },
                                p,
                                1.8,
                                None,
                            );
                        }
                    }
                    Elem::Midia { r, acao, tocando } => {
                        let cx = (r.left + r.right) / 2.0;
                        let cy = (r.top + r.bottom) / 2.0;
                        if hover == Some(*acao) {
                            p.SetColor(&crate::tema::tinta(0.12));
                            dc.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: cx, y: cy },
                                    radiusX: 16.0,
                                    radiusY: 16.0,
                                },
                                p,
                            );
                        }
                        p.SetColor(&crate::tema::tinta(0.92));
                        match acao {
                            Acao::TocarPausar if *tocando => icones::pausar(dc, p, cx, cy, 0.9),
                            Acao::TocarPausar => icones::tocar(dc, p, cx + 1.0, cy, 0.9),
                            Acao::Anterior => icones::pular(dc, p, cx, cy, 0.75, false),
                            _ => icones::pular(dc, p, cx, cy, 0.75, true),
                        }
                    }
                }
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}
