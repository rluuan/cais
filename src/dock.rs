//! O dock: ícones reais, ampliação em onda, nome do app, pulo ao abrir,
//! arrastar para reordenar, ocultar automaticamente, menu de contexto e
//! vidro fosco no formato exato do fundo.

use crate::apps::{self, Modelo};
use crate::gpu::Gpu;
use crate::janela;
use crate::medidor::QUADROS;
use crate::mola::Mola;
use crate::superficie::{arredondado, retangulo, Superficie};
use crate::vidro::Vidro;
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::time::Instant;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

const ICONE: f32 = 50.0;
const ESPACO: f32 = 6.0;
const MARGEM_LADO: f32 = 8.0;
const MARGEM_TOPO: f32 = 6.0;
const MARGEM_BAIXO: f32 = 10.0;
const ALT_FUNDO: f32 = ICONE + MARGEM_TOPO + MARGEM_BAIXO;
const RAIO_FUNDO: f32 = 18.0;
pub const AMPLIACAO: f32 = 1.75;
const ALCANCE: f32 = ICONE * 2.8;
const LARG_SEPARADOR: f32 = 13.0;
const ALT_JANELA: f32 = 176.0;
/// Dock de lado: a janela é mais larga para o nome do app caber ao lado.
const ALT_JANELA_LADO: f32 = 420.0;

/// Onde o dock fica no monitor principal.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Posicao {
    Baixo,
    Esquerda,
    Direita,
}

impl Posicao {
    pub fn de(s: &str) -> Self {
        match s {
            "esquerda" => Posicao::Esquerda,
            "direita" => Posicao::Direita,
            _ => Posicao::Baixo,
        }
    }
}

/// Converte um ponto do dock "deitado" (x ao longo do dock, y da parte de
/// dentro da tela até a borda) para a janela do dock na posição real.
pub fn para_tela(p: Posicao, espessura: f32, lx: f32, ly: f32) -> (f32, f32) {
    match p {
        Posicao::Baixo => (lx, ly),
        Posicao::Esquerda => (espessura - ly, lx),
        Posicao::Direita => (ly, lx),
    }
}

/// O inverso de `para_tela`.
pub fn para_logico_em(p: Posicao, espessura: f32, sx: f32, sy: f32) -> (f32, f32) {
    match p {
        Posicao::Baixo => (sx, sy),
        Posicao::Esquerda => (sy, espessura - sx),
        Posicao::Direita => (sy, sx),
    }
}
const DISTANCIA_TELA: f32 = 6.0;
const PULO_ALTURA: f32 = 16.0;
const PULO_DURACAO: f32 = 0.45;
const PULOS: f32 = 2.0;
const DISTANCIA_REMOVER: f32 = 60.0;

/// Timer que esconde o dock um instante depois que o mouse sai.
pub const TIMER_OCULTAR: usize = 4;
/// Mensagem que o Windows manda ao dock (tela cheia, barras mudaram).
pub const WM_APPBAR: u32 = WM_APP + 1;

pub enum AcaoMenu {
    Nada,
    Sair,
    AlternarAuto,
    AlternarBarra,
    /// Para onde minimizar: 0 = lateral, 1 = dock, 2 = não mexer.
    Minimizar(u8),
    Preferencias,
    Posicao(Posicao),
    AbrirPasta(std::path::PathBuf),
}

struct Disposicao {
    fundo: D2D_RECT_F,
    /// (centro x, tamanho, base y) de cada ícone, em DIPs
    icones: Vec<(f32, f32, f32)>,
    /// x de cada separador (entre grupos).
    separadores: Vec<f32>,
}

/// Janela minimizada que mora no dock (à direita), com miniatura ao vivo
/// desenhada pelo compositor do Windows (nada é copiado pela CPU).
struct Mini {
    hwnd: HWND,
    mola: Mola,
    miniatura: isize,
    origem: (f32, f32),
    titulo: String,
}

struct Arrasto {
    indice: usize,
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    ativo: bool,
}

pub struct Dock {
    sup: Superficie,
    vidro: Vidro,
    pub modelo: Modelo,
    minis: Vec<Mini>,
    pub minimizar_no_dock: bool,
    /// 0 = lateral, 1 = dock, 2 = não mexer (só para marcar no menu).
    pub modo_minimizar: u8,
    /// Quanto o ícone cresce sob o mouse (1 = não cresce).
    pub ampliacao: f32,
    /// Clicaram no ícone do Launchpad (a App abre/fecha).
    pub pedido_launchpad: bool,
    /// Clicaram no ícone do Finder.
    pub pedido_finder: bool,
    ponteiro: Option<f32>,
    fracao: f32,
    hover: Option<usize>,
    pressionado: Option<usize>,
    arrasto: Option<Arrasto>,
    /// 0 = visível, 1 = escondido embaixo da tela (ocultar automático).
    oculto: Mola,
    pub auto: bool,
    pub barra_escondida: bool,
    reservar: bool,
    ultima_reserva: Option<RECT>,
    pincel: ID2D1SolidColorBrush,
    formato: IDWriteTextFormat,
    larg_dip: f32,
    regiao_cheia: bool,
    regiao_oculta: bool,
    pub sujo: bool,
    pub posicao: Posicao,
    /// Clicaram numa pasta do dock: (pasta, retângulo do ícone na tela).
    pub pedido_pilha: Option<(std::path::PathBuf, RECT)>,
    /// A lixeira tem algo dentro (muda o desenho do ícone).
    lixeira_cheia: bool,
    /// Ícone com o foco do teclado (Ctrl+F3), None = fora do modo teclado.
    foco: Option<usize>,
}

fn px_icone(escala: f32) -> u32 {
    (ICONE * AMPLIACAO * escala).ceil() as u32
}

fn deslocamento_pulo(p: Option<Instant>) -> f32 {
    let Some(t) = p else { return 0.0 };
    let el = t.elapsed().as_secs_f32() / PULO_DURACAO;
    if el >= PULOS {
        return 0.0;
    }
    (el.fract() * PI).sin() * PULO_ALTURA * (1.0 - 0.3 * el.floor())
}

impl Dock {
    /// `reservar`: registra o dock no Windows para reservar espaço e saber de
    /// tela cheia (desligado no modo de medição, para não mexer nas janelas).
    pub fn new(gpu: &Gpu, auto: bool, barra_escondida: bool, reservar: bool, posicao: Posicao) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)?;
        janela::preparar_vidro(hwnd);
        if reservar {
            janela::registrar_appbar(hwnd, WM_APPBAR);
        }
        let mut ultima_reserva = None;
        posicionar_janela(hwnd, reservar, auto, &mut ultima_reserva, posicao);
        let sup = Superficie::new(gpu, hwnd)?;
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, crate::vidro::TINTA_DOCK)?;
        let escala = sup.dpi / 96.0;
        let mut modelo = Modelo::carregar();
        modelo.carregar_icones(&gpu.d2d_dc, px_icone(escala));
        let (lc, ac) = janela::tamanho_cliente(hwnd);
        let l = if posicao == Posicao::Baixo { lc } else { ac };
        let mut oculto = Mola::default();
        if auto {
            oculto.x = 1.0;
            oculto.alvo = 1.0;
        }
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
            let mut d = Self {
                sup,
                vidro,
                modelo,
                minis: Vec::new(),
                minimizar_no_dock: true,
                modo_minimizar: 0,
                ampliacao: AMPLIACAO,
                pedido_launchpad: false,
                pedido_finder: false,
                ponteiro: None,
                fracao: 0.5,
                hover: None,
                pressionado: None,
                arrasto: None,
                oculto,
                auto,
                barra_escondida,
                reservar,
                ultima_reserva,
                pincel,
                formato,
                larg_dip: l as f32 / escala,
                regiao_cheia: false,
                regiao_oculta: false,
                sujo: true,
                posicao,
                pedido_pilha: None,
                lixeira_cheia: crate::arquivos::lixeira_tem_algo(),
                foco: None,
            };
            d.atualizar_regiao(true);
            Ok(d)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    pub fn mostrar(&self) {
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Um app entrou (ou saiu) de tela cheia: o dock some (ou volta).
    pub fn definir_tela_cheia(&self, cheia: bool) {
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, if cheia { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
    }

    /// Muda o dock de lugar (embaixo, esquerda ou direita).
    pub fn definir_posicao(&mut self, gpu: &Gpu, p: Posicao) {
        if p == self.posicao {
            return;
        }
        self.posicao = p;
        self.ultima_reserva = None;
        // Devolve a reserva da borda antiga antes de pedir a nova.
        if self.reservar {
            janela::remover_appbar(self.sup.hwnd);
            janela::registrar_appbar(self.sup.hwnd, WM_APPBAR);
        }
        let _ = self.reposicionar(gpu);
    }

    /// Espessura da janela do dock (DIPs), no sentido perpendicular a ele.
    fn espessura(&self) -> f32 {
        if self.posicao == Posicao::Baixo {
            ALT_JANELA
        } else {
            ALT_JANELA_LADO
        }
    }

    /// Coordenadas da janela (DIPs) para as do dock "deitado".
    pub fn para_logico(&self, x: f32, y: f32) -> (f32, f32) {
        para_logico_em(self.posicao, self.espessura(), x, y)
    }

    fn tela(&self, x: f32, y: f32) -> (f32, f32) {
        para_tela(self.posicao, self.espessura(), x, y)
    }

    /// Retângulo do dock deitado para a janela (DIPs).
    fn ret_tela(&self, r: D2D_RECT_F) -> D2D_RECT_F {
        let (x0, y0) = self.tela(r.left, r.top);
        let (x1, y1) = self.tela(r.right, r.bottom);
        retangulo(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
    }

    pub fn reposicionar(&mut self, gpu: &Gpu) -> Result<()> {
        posicionar_janela(
            self.sup.hwnd,
            self.reservar,
            self.auto,
            &mut self.ultima_reserva,
            self.posicao,
        );
        self.sup.redimensionar(gpu)?;
        let (lc, ac) = janela::tamanho_cliente(self.sup.hwnd);
        let l = if self.posicao == Posicao::Baixo { lc } else { ac };
        self.larg_dip = l as f32 / self.escala();
        self.atualizar_regiao(true);
        self.sujo = true;
        Ok(())
    }

    /// Explorer reiniciou: o registro de barra de aplicativo se perdeu.
    pub fn registrar_de_novo(&mut self, gpu: &Gpu) {
        if self.reservar {
            janela::registrar_appbar(self.sup.hwnd, WM_APPBAR);
            self.ultima_reserva = None;
        }
        let _ = self.reposicionar(gpu);
    }

    pub fn sair_do_windows(&self) {
        if self.reservar {
            janela::remover_appbar(self.sup.hwnd);
        }
    }

    pub fn definir_auto(&mut self, gpu: &Gpu, auto: bool) {
        self.auto = auto;
        self.oculto.alvo = if auto && self.ponteiro.is_none() { 1.0 } else { 0.0 };
        let _ = self.reposicionar(gpu);
    }

    /// Relê os apps abertos (chamado quando o Windows avisa que uma janela
    /// apareceu, sumiu ou mudou).
    pub fn atualizar_apps(&mut self, gpu: &Gpu) {
        if self.arrasto.is_some() {
            return; // não bagunça a ordem no meio de um arraste
        }
        self.modelo.atualizar_janelas();
        self.modelo
            .carregar_icones(&gpu.d2d_dc, px_icone(self.escala()));
        self.lixeira_cheia = crate::arquivos::lixeira_tem_algo();
        self.conferir_minis();
        self.hover = None;
        self.pressionado = None;
        if self.ponteiro.is_some() {
            self.definir_ponteiro(self.ponteiro);
        }
        self.atualizar_regiao(true);
        self.sujo = true;
    }

    /// Linha de base do dock (DIPs): desce para fora da tela quando oculto.
    fn chao(&self) -> f32 {
        self.espessura() - DISTANCIA_TELA + self.oculto.x * (ALT_FUNDO + DISTANCIA_TELA + 6.0)
    }

    fn dispor(&self, ampliado: bool) -> Disposicao {
        let itens = &self.modelo.itens;
        let ni = itens.len();
        let n = ni + self.minis.len();
        let nf = self.modelo.qtd_fixados();
        let nd = self.modelo.qtd_direita();
        // Separadores: fixados | abertos | pastas e lixeira | minimizadas.
        let tem_sep = nf > 0 && nf < ni - nd;
        let tem_sep_dir = nd > 0 && ni > nd;
        let tem_sep_mini = ni > 0 && !self.minis.is_empty();
        let espacos = n.saturating_sub(1) as f32 * ESPACO;
        let sep = (tem_sep as u8 + tem_sep_dir as u8 + tem_sep_mini as u8) as f32 * LARG_SEPARADOR;
        let larg_repouso = MARGEM_LADO * 2.0 + n as f32 * ICONE + espacos + sep;
        let tams: Vec<f32> = itens
            .iter()
            .map(|i| &i.mola)
            .chain(self.minis.iter().map(|m| &m.mola))
            .map(|m| {
                if ampliado {
                    ICONE * (1.0 + (self.ampliacao - 1.0) * m.x)
                } else {
                    ICONE
                }
            })
            .collect();
        let larg = MARGEM_LADO * 2.0 + tams.iter().sum::<f32>() + espacos + sep;
        // O dock cresce para os dois lados na proporção de onde o mouse está,
        // então o ícone embaixo do cursor quase não sai do lugar.
        let esq = (self.larg_dip - larg_repouso) / 2.0 - (larg - larg_repouso) * self.fracao;
        let chao = self.chao();
        let mut x = esq + MARGEM_LADO;
        let mut icones = Vec::with_capacity(n);
        let mut separadores = Vec::new();
        for (i, &t) in tams.iter().enumerate() {
            if (tem_sep && i == nf) || (tem_sep_dir && i == ni - nd) || (tem_sep_mini && i == ni) {
                separadores.push(x + (LARG_SEPARADOR - ESPACO) / 2.0);
                x += LARG_SEPARADOR;
            }
            let pulo = if ampliado && i < ni {
                deslocamento_pulo(itens[i].pulo)
            } else {
                0.0
            };
            icones.push((x + t / 2.0, t, chao - MARGEM_BAIXO - pulo));
            x += t + ESPACO;
        }
        Disposicao {
            fundo: retangulo(esq, chao - ALT_FUNDO, esq + larg, chao),
            icones,
            separadores,
        }
    }

    // ------------------------------------------------------------------
    // Janelas minimizadas

    /// Uma janela foi minimizada: ganha uma miniatura no fim do dock.
    pub fn minimizou(&mut self, h: HWND) {
        if !self.minimizar_no_dock
            || self.minis.iter().any(|m| m.hwnd == h)
            || !apps::eh_janela_de_app(h, true)
        {
            return;
        }
        unsafe {
            let Ok(miniatura) = DwmRegisterThumbnail(self.sup.hwnd, h) else {
                return;
            };
            let origem = DwmQueryThumbnailSourceSize(miniatura)
                .map(|s| (s.cx.max(1) as f32, s.cy.max(1) as f32))
                .unwrap_or((16.0, 10.0));
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, &mut buf) as usize;
            self.minis.push(Mini {
                hwnd: h,
                mola: Mola::default(),
                miniatura,
                origem,
                titulo: String::from_utf16_lossy(&buf[..n.min(buf.len())]),
            });
        }
        self.depois_de_mudar_minis();
    }

    /// Voltou ao tamanho normal (ou fechou): a miniatura sai do dock.
    pub fn restaurou(&mut self, h: HWND) {
        let antes = self.minis.len();
        self.minis.retain(|m| {
            if m.hwnd == h {
                unsafe {
                    let _ = DwmUnregisterThumbnail(m.miniatura);
                }
                false
            } else {
                true
            }
        });
        if self.minis.len() != antes {
            self.depois_de_mudar_minis();
        }
    }

    /// Ao abrir o Cais: o que já estava minimizado também entra.
    pub fn ler_minimizadas(&mut self) {
        for (h, _) in apps::enumerar_janelas() {
            if unsafe { IsIconic(h).as_bool() } {
                self.minimizou(h);
            }
        }
    }

    /// Tira miniaturas de janelas que sumiram ou foram restauradas por fora.
    fn conferir_minis(&mut self) {
        let mortas: Vec<HWND> = self
            .minis
            .iter()
            .filter(|m| unsafe { !IsWindow(m.hwnd).as_bool() || !IsIconic(m.hwnd).as_bool() })
            .map(|m| m.hwnd)
            .collect();
        for h in mortas {
            self.restaurou(h);
        }
    }

    pub fn definir_minimizar_no_dock(&mut self, sim: bool) {
        self.minimizar_no_dock = sim;
        if sim {
            self.ler_minimizadas();
        } else {
            let todas: Vec<HWND> = self.minis.iter().map(|m| m.hwnd).collect();
            for h in todas {
                self.restaurou(h);
            }
        }
    }

    fn depois_de_mudar_minis(&mut self) {
        self.hover = None;
        self.pressionado = None;
        if self.ponteiro.is_some() {
            self.definir_ponteiro(self.ponteiro);
        }
        self.atualizar_regiao(true);
        self.sujo = true;
    }

    fn eh_mini(&self, i: usize) -> Option<usize> {
        i.checked_sub(self.modelo.itens.len()).filter(|&k| k < self.minis.len())
    }

    fn restaurar_mini(&mut self, k: usize) {
        let h = self.minis[k].hwnd;
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        self.restaurou(h);
    }

    /// Coloca cada miniatura no lugar do seu "ícone", mantendo a proporção
    /// da janela. Quem desenha é o DWM, por cima do nosso quadro.
    fn posicionar_minis(&self, d: &Disposicao) {
        let e = self.escala();
        let ni = self.modelo.itens.len();
        let visivel = self.oculto.x < 0.98;
        for (k, m) in self.minis.iter().enumerate() {
            let Some(&(cx, t, base)) = d.icones.get(ni + k) else { continue };
            let lado = t * 0.92;
            let (w, h) = m.origem;
            let (lw, lh) = if w >= h { (lado, lado * h / w) } else { (lado * w / h, lado) };
            let cy = base - t / 2.0;
            // O centro gira com o dock; a miniatura fica sempre de pé.
            let (sx, sy) = self.tela(cx, cy);
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                rcDestination: RECT {
                    left: ((sx - lw / 2.0) * e).round() as i32,
                    top: ((sy - lh / 2.0) * e).round() as i32,
                    right: ((sx + lw / 2.0) * e).round() as i32,
                    bottom: ((sy + lh / 2.0) * e).round() as i32,
                },
                rcSource: RECT::default(),
                opacity: if self.pressionado == Some(ni + k) { 160 } else { 255 },
                fVisible: visivel.into(),
                fSourceClientAreaOnly: false.into(),
            };
            unsafe {
                let _ = DwmUpdateThumbnailProperties(m.miniatura, &props);
            }
        }
    }

    /// Posição do mouse em DIPs (None = mouse saiu). Os vizinhos também
    /// crescem, numa curva suave, como num dock de verdade.
    pub fn definir_ponteiro(&mut self, x: Option<f32>) {
        self.ponteiro = x;
        let repouso = self.dispor(false);
        if let Some(x) = x {
            let f = repouso.fundo;
            self.fracao = ((x - f.left) / (f.right - f.left).max(1.0)).clamp(0.0, 1.0);
        }
        let alvo = |i: usize| match x {
            None => 0.0,
            Some(x) => {
                let d = (repouso.icones[i].0 - x).abs() / ALCANCE;
                if d >= 1.0 {
                    0.0
                } else {
                    (1.0 + (PI * d).cos()) / 2.0
                }
            }
        };
        let ni = self.modelo.itens.len();
        for (i, it) in self.modelo.itens.iter_mut().enumerate() {
            it.mola.alvo = alvo(i);
        }
        for (k, m) in self.minis.iter_mut().enumerate() {
            m.mola.alvo = alvo(ni + k);
        }
        self.sujo = true;
        self.atualizar_regiao(false);
    }

    fn acertar(&self, x: f32, y: f32) -> Option<usize> {
        let d = self.dispor(true);
        d.icones
            .iter()
            .position(|&(cx, t, base)| (x - cx).abs() <= t / 2.0 + ESPACO / 2.0 && y >= base - t - 2.0)
    }

    pub fn mover(&mut self, x: f32, y: f32) {
        if self.arrastar(x, y) {
            return;
        }
        self.cancelar_ocultar();
        if self.auto && self.oculto.alvo > 0.5 {
            // Encostou na borda de baixo: o dock sobe.
            self.oculto.alvo = 0.0;
            self.atualizar_regiao(true);
            self.sujo = true;
            return;
        }
        let d = self.dispor(true);
        let f = d.fundo;
        let no_fundo = x >= f.left && x <= f.right && y >= f.top;
        let topo_zona = self.chao() - MARGEM_BAIXO - ICONE * self.ampliacao - 8.0;
        let na_zona =
            self.ponteiro.is_some() && x >= f.left - 4.0 && x <= f.right + 4.0 && y >= topo_zona;
        if no_fundo || na_zona {
            self.definir_ponteiro(Some(x));
            let novo = self.acertar(x, y);
            if novo != self.hover {
                self.hover = novo;
                self.sujo = true;
            }
        } else {
            self.sair();
        }
    }

    pub fn sair(&mut self) {
        if self.arrasto.as_ref().is_some_and(|a| a.ativo) {
            return;
        }
        if self.ponteiro.is_some() || self.hover.is_some() || self.pressionado.is_some() {
            self.hover = None;
            self.pressionado = None;
            self.definir_ponteiro(None);
        }
        if self.auto {
            unsafe {
                SetTimer(self.sup.hwnd, TIMER_OCULTAR, 450, None);
            }
        }
    }

    fn cancelar_ocultar(&self) {
        if self.auto {
            unsafe {
                let _ = KillTimer(self.sup.hwnd, TIMER_OCULTAR);
            }
        }
    }

    /// Chamado pelo timer: se o mouse não voltou, o dock desce.
    pub fn ocultar_se_ocioso(&mut self) {
        unsafe {
            let _ = KillTimer(self.sup.hwnd, TIMER_OCULTAR);
        }
        if self.auto && self.ponteiro.is_none() && self.arrasto.is_none() {
            self.oculto.alvo = 1.0;
            self.atualizar_regiao(true);
            self.sujo = true;
        }
    }

    pub fn botao_baixo(&mut self, x: f32, y: f32) {
        self.pressionado = self.acertar(x, y);
        if let Some(i) = self.pressionado.filter(|&i| self.eh_mini(i).is_none()) {
            self.arrasto = Some(Arrasto {
                indice: i,
                x0: x,
                y0: y,
                x,
                y,
                ativo: false,
            });
            unsafe {
                SetCapture(self.sup.hwnd);
            }
        }
        self.sujo = true;
    }

    /// Devolve true se o movimento foi consumido pelo arraste.
    fn arrastar(&mut self, x: f32, y: f32) -> bool {
        let Some(a) = self.arrasto.as_mut() else {
            return false;
        };
        a.x = x;
        a.y = y;
        if !a.ativo {
            let dx = x - a.x0;
            let dy = y - a.y0;
            if dx * dx + dy * dy < 36.0 {
                return false;
            }
            a.ativo = true;
            self.pressionado = None;
            self.hover = None;
        }
        self.reordenar(x);
        let f = self.dispor(false).fundo;
        self.definir_ponteiro(Some(x.clamp(f.left, f.right)));
        self.sujo = true;
        true
    }

    /// Troca o item arrastado de lugar com o vizinho quando o cursor passa do
    /// centro dele. Só dentro do mesmo grupo (fixados ou abertos).
    fn reordenar(&mut self, x: f32) {
        let Some(k) = self.arrasto.as_ref().map(|a| a.indice) else {
            return;
        };
        let n = self.modelo.itens.len();
        let nf = self.modelo.qtd_fixados();
        let nd = n - self.modelo.qtd_direita();
        let (g0, g1) = if k < nf {
            (0, nf)
        } else if k < nd {
            (nf, nd)
        } else {
            (nd, n)
        };
        let d = self.dispor(true);
        if k > g0 && x < d.icones[k - 1].0 {
            self.trocar(k - 1, &d);
            self.arrasto.as_mut().unwrap().indice = k - 1;
        } else if k + 1 < g1 && x > d.icones[k + 1].0 {
            self.trocar(k, &d);
            self.arrasto.as_mut().unwrap().indice = k + 1;
        }
    }

    /// Troca os itens i e i+1. O vizinho desliza suavemente até o lugar novo
    /// (começa onde estava e uma mola leva o deslocamento até zero).
    fn trocar(&mut self, i: usize, antes: &Disposicao) {
        let visual: Vec<f32> = [i, i + 1]
            .iter()
            .map(|&k| antes.icones[k].0 + self.modelo.itens[k].desloc.x)
            .collect();
        self.modelo.itens.swap(i, i + 1);
        let depois = self.dispor(true);
        // o que estava em i agora está em i+1 e vice-versa
        for (pos_nova, x_antes) in [(i + 1, visual[0]), (i, visual[1])] {
            let m = &mut self.modelo.itens[pos_nova].desloc;
            m.x = x_antes - depois.icones[pos_nova].0;
            m.alvo = 0.0;
        }
    }

    fn zona_remover(&self, y: f32) -> bool {
        y < self.chao() - ALT_FUNDO - DISTANCIA_REMOVER
    }

    pub fn botao_cima(&mut self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Some(a) = self.arrasto.take() {
            if a.ativo {
                let k = a.indice;
                if self.zona_remover(y) && self.modelo.itens[k].fixado {
                    self.modelo.desafixar(k);
                } else {
                    self.modelo.salvar();
                }
                self.pressionado = None;
                self.mover(x, y);
                self.atualizar_regiao(true);
                self.sujo = true;
                return;
            }
        }
        let alvo = self.acertar(x, y);
        let antes = self.pressionado.take();
        self.sujo = true;
        if let Some(i) = alvo.filter(|_| alvo == antes) {
            self.acionar_indice(i);
        }
    }

    /// Navegação pelo teclado (Ctrl+F3): ícone com o foco.
    pub fn foco_teclado(&self) -> Option<usize> {
        self.foco
    }

    pub fn definir_foco(&mut self, f: Option<usize>) {
        let n = self.modelo.itens.len() + self.minis.len();
        self.foco = f.filter(|_| n > 0).map(|i| i.min(n - 1));
        self.sujo = true;
    }

    /// Anda com o foco do teclado (passo +1 ou -1, dá a volta).
    pub fn mover_foco(&mut self, passo: isize) {
        let n = (self.modelo.itens.len() + self.minis.len()) as isize;
        if n == 0 {
            return;
        }
        let atual = self.foco.unwrap_or(0) as isize;
        self.foco = Some((atual + passo).rem_euclid(n) as usize);
        self.sujo = true;
    }

    /// O mesmo que clicar no ícone i.
    pub fn acionar_indice(&mut self, i: usize) {
        {
            if let Some(k) = self.eh_mini(i) {
                self.restaurar_mini(k);
            } else if self.modelo.itens[i].caminho == apps::LIXEIRA {
                crate::arquivos::abrir_lixeira();
                self.pular(i);
            } else if self.modelo.itens[i].eh_pasta() {
                let pasta = std::path::PathBuf::from(&self.modelo.itens[i].caminho);
                self.pedido_pilha = Some((pasta, self.ret_icone_na_tela(i)));
            } else if self.modelo.itens[i].caminho == apps::LAUNCHPAD {
                self.pedido_launchpad = true;
            } else if self.modelo.itens[i].caminho == apps::FINDER {
                self.pedido_finder = true;
            } else if apps::acionar(&self.modelo.itens[i]) {
                self.pular(i);
            }
        }
    }

    /// O Windows tirou o mouse de nós no meio de um arraste (Alt+Tab etc.).
    pub fn cancelar_arrasto(&mut self) {
        if self.arrasto.take().is_some_and(|a| a.ativo) {
            self.modelo.salvar();
        }
        self.pressionado = None;
        self.sujo = true;
    }

    /// Retângulo do ícone i na tela (px), para a prévia e as pilhas.
    fn ret_icone_na_tela(&self, i: usize) -> RECT {
        let d = self.dispor(true);
        let e = self.escala();
        let Some(&(cx, t, base)) = d.icones.get(i) else { return RECT::default() };
        let r = self.ret_tela(retangulo(cx - t / 2.0, base - t, cx + t / 2.0, base));
        let mut w = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut w);
        }
        RECT {
            left: w.left + (r.left * e) as i32,
            top: w.top + (r.top * e) as i32,
            right: w.left + (r.right * e) as i32,
            bottom: w.top + (r.bottom * e) as i32,
        }
    }

    /// Item do dock sob o mouse (para saber quando mostrar a prévia).
    pub fn item_sob_mouse(&self) -> Option<usize> {
        self.hover.filter(|&i| self.eh_mini(i).is_none())
    }

    /// O mouse continua sobre algum ícone do dock?
    pub fn mouse_no_item_da_previa(&self) -> bool {
        self.ponteiro.is_some() && self.item_sob_mouse().is_some()
    }

    /// Janelas do app sob o mouse e onde fica o ícone (para a prévia).
    pub fn pedido_de_previa(&self) -> Option<(Vec<HWND>, RECT)> {
        let i = self.item_sob_mouse()?;
        let it = self.modelo.itens.get(i)?;
        if it.janelas.is_empty() || self.arrasto.is_some() {
            return None;
        }
        Some((it.janelas.clone(), self.ret_icone_na_tela(i)))
    }

    fn pular(&mut self, i: usize) {
        self.modelo.itens[i].pulo = Some(Instant::now());
        self.atualizar_regiao(false);
    }

    /// Menu do clique direito.
    pub fn menu(&mut self, x: f32, y: f32) -> AcaoMenu {
        let alvo = self.acertar(x, y);
        if let Some(k) = alvo.and_then(|i| self.eh_mini(i)) {
            return self.menu_mini(k);
        }
        let hwnd = self.sup.hwnd;
        let marcado = |b: bool| if b { MF_STRING | MF_CHECKED } else { MF_STRING };
        let t = crate::idioma::t;
        let mut recentes: Vec<std::path::PathBuf> = Vec::new();
        let cmd = unsafe {
            let Ok(m) = CreatePopupMenu() else {
                return AcaoMenu::Nada;
            };
            let item = |m: HMENU, flags: MENU_ITEM_FLAGS, id: usize, s: &str| {
                let w = crate::idioma::largo(s);
                let _ = AppendMenuW(m, flags, id, PCWSTR(w.as_ptr()));
            };
            if let Some(i) = alvo {
                let it = &self.modelo.itens[i];
                if it.caminho == apps::LIXEIRA {
                    item(m, MF_STRING, 1, t("Abrir", "Open"));
                    let flags = if self.lixeira_cheia { MF_STRING } else { MF_STRING | MF_GRAYED };
                    item(m, flags, 5, t("Esvaziar Lixeira…", "Empty Trash…"));
                } else if it.eh_pasta() {
                    item(m, MF_STRING, 1, t("Abrir no Arquivos", "Open in Files"));
                    item(m, MF_STRING, 3, t("Remover do Cais", "Remove from Cais"));
                } else {
                    // Arquivos usados há pouco com este app (lista "Recentes"
                    // do próprio Windows).
                    if !it.exe.is_empty() && !apps::eh_especial(&it.caminho) {
                        recentes = apps::recentes_do_app(&it.exe, 8);
                    }
                    if !recentes.is_empty() {
                        if let Ok(sub) = CreatePopupMenu() {
                            for (k, r) in recentes.iter().enumerate() {
                                let nome = r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                item(sub, MF_STRING, 100 + k, &nome);
                            }
                            item(m, MF_POPUP, sub.0 as usize, t("Recentes", "Recents"));
                            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
                        }
                    }
                    let abrir = if it.janelas.is_empty() { t("Abrir", "Open") } else { t("Nova janela", "New window") };
                    item(m, MF_STRING, 1, abrir);
                    if it.fixado {
                        item(m, MF_STRING, 3, t("Remover do Cais", "Remove from Cais"));
                    } else {
                        item(m, MF_STRING, 2, t("Manter no Cais", "Keep in Cais"));
                    }
                    // Rodando mesmo sem janela (só na bandeja) também fecha.
                    let explorador = it.exe.to_lowercase().ends_with(r"\explorer.exe");
                    let rodando = !explorador && !apps::eh_especial(&it.caminho) && !apps::processos_do_app(it).is_empty();
                    if !it.janelas.is_empty() || rodando {
                        item(m, MF_STRING, 4, t("Fechar", "Quit"));
                    }
                    if rodando {
                        item(m, MF_STRING, 6, t("Forçar encerramento", "Force Quit"));
                    }
                }
                let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            }
            item(m, marcado(self.auto), 10, t("Ocultar automaticamente", "Automatically hide"));
            item(m, marcado(self.barra_escondida), 11, t("Esconder a barra de tarefas do Windows", "Hide the Windows taskbar"));
            if let Ok(sub) = CreatePopupMenu() {
                let opcoes = [
                    (30usize, Posicao::Esquerda, t("Esquerda", "Left")),
                    (31, Posicao::Baixo, t("Embaixo", "Bottom")),
                    (32, Posicao::Direita, t("Direita", "Right")),
                ];
                for (id, p, texto) in opcoes {
                    item(sub, marcado(self.posicao == p), id, texto);
                }
                item(m, MF_POPUP, sub.0 as usize, t("Posição na tela", "Position on screen"));
            }
            if let Ok(sub) = CreatePopupMenu() {
                let opcoes = [
                    (20usize, t("Telinha no Palco (lateral)", "Thumbnail on the Stage (side)")),
                    (21, t("Miniatura no fim do dock", "Thumbnail at the end of the dock")),
                    (22, t("Não mexer (como o Windows)", "Leave it (like Windows)")),
                ];
                for (k, (id, texto)) in opcoes.into_iter().enumerate() {
                    item(sub, marcado(self.modo_minimizar == k as u8), id, texto);
                }
                item(m, MF_POPUP, sub.0 as usize, t("Ao minimizar janelas", "When minimizing windows"));
            }
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            item(m, MF_STRING, 13, t("Preferências do Cais…", "Cais Settings…"));
            item(m, MF_STRING, 9, t("Sair do Cais", "Quit Cais"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let alinhar = match self.posicao {
                Posicao::Baixo => TPM_BOTTOMALIGN | TPM_CENTERALIGN,
                Posicao::Esquerda => TPM_LEFTALIGN | TPM_VCENTERALIGN,
                Posicao::Direita => TPM_RIGHTALIGN | TPM_VCENTERALIGN,
            };
            let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | alinhar, pt.x, pt.y, 0, hwnd, None).0;
            let _ = DestroyMenu(m);
            cmd
        };
        let mut acao = AcaoMenu::Nada;
        match (cmd, alvo) {
            (9, _) => return AcaoMenu::Sair,
            (13, _) => acao = AcaoMenu::Preferencias,
            (10, _) => acao = AcaoMenu::AlternarAuto,
            (11, _) => acao = AcaoMenu::AlternarBarra,
            (c @ 20..=22, _) => acao = AcaoMenu::Minimizar((c - 20) as u8),
            (30, _) => acao = AcaoMenu::Posicao(Posicao::Esquerda),
            (31, _) => acao = AcaoMenu::Posicao(Posicao::Baixo),
            (32, _) => acao = AcaoMenu::Posicao(Posicao::Direita),
            (c @ 100..=107, _) => {
                if let Some(r) = recentes.get(c as usize - 100) {
                    apps::abrir(&r.to_string_lossy());
                }
            }
            (5, _) => crate::arquivos::esvaziar_lixeira(hwnd),
            (1, Some(i)) if self.modelo.itens[i].caminho == apps::LIXEIRA => crate::arquivos::abrir_lixeira(),
            (1, Some(i)) if self.modelo.itens[i].eh_pasta() => {
                acao = AcaoMenu::AbrirPasta(std::path::PathBuf::from(&self.modelo.itens[i].caminho));
            }
            (1, Some(i)) if self.modelo.itens[i].caminho == apps::LAUNCHPAD => {
                self.pedido_launchpad = true;
            }
            (1, Some(i)) if self.modelo.itens[i].caminho == apps::FINDER => {
                self.pedido_finder = true;
            }
            (1, Some(i)) => {
                apps::abrir(&self.modelo.itens[i].caminho.clone());
                self.pular(i);
            }
            (2, Some(i)) => self.modelo.fixar(i),
            (3, Some(i)) => self.modelo.desafixar(i),
            (4, Some(i)) => apps::fechar_app(&self.modelo.itens[i]),
            (6, Some(i)) => apps::forcar_encerramento(&apps::processos_do_app(&self.modelo.itens[i])),
            _ => {}
        }
        self.sair();
        self.atualizar_regiao(true);
        self.sujo = true;
        acao
    }

    fn menu_mini(&mut self, k: usize) -> AcaoMenu {
        let hwnd = self.sup.hwnd;
        let cmd = unsafe {
            let Ok(m) = CreatePopupMenu() else {
                return AcaoMenu::Nada;
            };
            let a = crate::idioma::largo(crate::idioma::t("Restaurar", "Restore"));
            let b = crate::idioma::largo(crate::idioma::t("Fechar janela", "Close window"));
            let _ = AppendMenuW(m, MF_STRING, 1, PCWSTR(a.as_ptr()));
            let _ = AppendMenuW(m, MF_STRING, 2, PCWSTR(b.as_ptr()));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let alinhar = match self.posicao {
                Posicao::Baixo => TPM_BOTTOMALIGN | TPM_CENTERALIGN,
                Posicao::Esquerda => TPM_LEFTALIGN | TPM_VCENTERALIGN,
                Posicao::Direita => TPM_RIGHTALIGN | TPM_VCENTERALIGN,
            };
            let c = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | alinhar,
                pt.x,
                pt.y,
                0,
                hwnd,
                None,
            )
            .0;
            let _ = DestroyMenu(m);
            c
        };
        if k < self.minis.len() {
            match cmd {
                1 => self.restaurar_mini(k),
                2 => unsafe {
                    let _ = PostMessageW(self.minis[k].hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                },
                _ => {}
            }
        }
        self.sair();
        self.atualizar_regiao(true);
        self.sujo = true;
        AcaoMenu::Nada
    }

    pub fn animando(&self) -> bool {
        self.minis.iter().any(|m| !m.mola.parada())
            || !self.oculto.parada()
            || self
                .modelo
                .itens
                .iter()
                .any(|i| !i.mola.parada() || !i.desloc.parada() || i.pulo.is_some())
    }

    pub fn passo(&mut self, dt: f32) {
        if !self.animando() {
            return;
        }
        let limite = PULO_DURACAO * PULOS;
        self.oculto.passo(dt);
        for m in self.minis.iter_mut() {
            m.mola.passo(dt);
        }
        for it in self.modelo.itens.iter_mut() {
            it.mola.passo(dt);
            it.desloc.passo(dt);
            if it.pulo.is_some_and(|t| t.elapsed().as_secs_f32() >= limite) {
                it.pulo = None;
            }
        }
        self.sujo = true;
    }

    /// Fora do dock o clique atravessa para a janela de baixo. Enquanto o
    /// mouse está no dock (ou algo anima), a janela inteira recebe o mouse.
    /// Oculto, sobra só uma faixa de 2 px na borda da tela para chamá-lo.
    pub fn atualizar_regiao(&mut self, forcar: bool) {
        let cheia = self.ponteiro.is_some() || self.arrasto.is_some() || self.animando();
        let oculta = !cheia && self.auto && self.oculto.alvo > 0.5;
        if cheia == self.regiao_cheia && oculta == self.regiao_oculta && !forcar {
            return;
        }
        self.regiao_cheia = cheia;
        self.regiao_oculta = oculta;
        let hwnd = self.sup.hwnd;
        if cheia {
            janela::definir_regiao(hwnd, None);
            return;
        }
        let e = self.escala();
        let mut f = self.dispor(false).fundo;
        if oculta {
            // Escondido: sobra uma faixa de 2 px colada na borda da tela.
            let borda = self.espessura();
            f = retangulo(f.left, borda - 2.0 / e, f.right, borda);
        }
        let r = self.ret_tela(f);
        janela::definir_regiao(
            hwnd,
            Some(RECT {
                left: (r.left * e).floor() as i32,
                top: (r.top * e).floor() as i32,
                right: (r.right * e).ceil() as i32,
                bottom: (r.bottom * e).ceil() as i32,
            }),
        );
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let d = self.dispor(true);
        let e = self.escala();
        let f = self.ret_tela(d.fundo);
        let _ = self.vidro.definir(
            f.left * e,
            f.top * e,
            (f.right - f.left) * e,
            (f.bottom - f.top) * e,
            RAIO_FUNDO * e,
        );
        let posicao = self.posicao;
        let espessura = self.espessura();
        let tela = move |x: f32, y: f32| para_tela(posicao, espessura, x, y);
        let lixeira_cheia = self.lixeira_cheia;

        let arrastado = self
            .arrasto
            .as_ref()
            .filter(|a| a.ativo)
            .map(|a| (a.indice, a.x, a.y));
        let removendo = arrastado.is_some_and(|(k, _, y)| {
            self.zona_remover(y) && self.modelo.itens[k].fixado
        });

        // Nome do app em cima do ícone sob o mouse (ou "Remover" ao arrastar
        // um fixado para fora).
        let rotulo = match (self.ponteiro, self.hover, arrastado) {
            (_, _, Some((_, x, y))) if removendo => Some((
                crate::idioma::t("Remover", "Remove").encode_utf16().collect::<Vec<u16>>(),
                x,
                y - ICONE * 0.6,
            )),
            (Some(_), Some(i), None) if self.eh_mini(i).is_some() && i < d.icones.len() => {
                let (cx, t, base) = d.icones[i];
                let m = &self.minis[i - self.modelo.itens.len()];
                Some((m.titulo.encode_utf16().collect::<Vec<u16>>(), cx, base - t))
            }
            (Some(_), Some(i), None) if i < d.icones.len() => {
                let (cx, t, base) = d.icones[i];
                Some((
                    self.modelo.itens[i].nome.encode_utf16().collect::<Vec<u16>>(),
                    cx + self.modelo.itens[i].desloc.x,
                    base - t,
                ))
            }
            // Modo teclado: o nome do ícone com o foco.
            (None, _, None) if self.foco.is_some_and(|f| f < self.modelo.itens.len() && f < d.icones.len()) => {
                let f = self.foco.unwrap();
                let (cx, t, base) = d.icones[f];
                Some((self.modelo.itens[f].nome.encode_utf16().collect::<Vec<u16>>(), cx, base - t))
            }
            _ => None,
        };
        let foco = self.foco;
        let rotulo = match rotulo {
            Some((texto, cx, topo)) => {
                let layout =
                    unsafe { gpu.dwrite.CreateTextLayout(&texto, &self.formato, 600.0, 60.0)? };
                Some((layout, cx, topo))
            }
            None => None,
        };

        let itens = &self.modelo.itens;
        let pincel = &self.pincel;
        let pressionado = self.pressionado;
        let larg_dip = self.larg_dip;
        let chao = self.chao();
        self.sup.desenhar(gpu, |dc| unsafe {
            // Filete claro na borda do vidro.
            let borda = arredondado(
                retangulo(f.left + 0.5, f.top + 0.5, f.right - 0.5, f.bottom - 0.5),
                RAIO_FUNDO - 0.5,
            );
            pincel.SetColor(&crate::tema::tinta(0.16));
            dc.DrawRoundedRectangle(&borda, pincel, 1.0, None);

            let fl = d.fundo;
            for &sx in &d.separadores {
                pincel.SetColor(&crate::tema::tinta(0.22));
                let (x0, y0) = tela(sx, fl.top + 13.0);
                let (x1, y1) = tela(sx, fl.bottom - 13.0);
                dc.DrawLine(D2D_POINT_2F { x: x0, y: y0 }, D2D_POINT_2F { x: x1, y: y1 }, pincel, 1.0, None);
            }

            // Ícones sempre de pé: só o centro gira com o dock.
            let desenhar_icone = |i: usize, cx: f32, t: f32, base: f32, opacidade: f32| {
                let (sx, sy) = tela(cx, base - t / 2.0);
                let r = retangulo(sx - t / 2.0, sy - t / 2.0, sx + t / 2.0, sy + t / 2.0);
                match &itens[i].icone {
                    Some(b) => dc.DrawBitmap(
                        b,
                        Some(&r),
                        opacidade,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    ),
                    None if itens[i].caminho == apps::LAUNCHPAD => {
                        crate::icones::launchpad(dc, pincel, r, opacidade);
                    }
                    None if itens[i].caminho == apps::FINDER => {
                        crate::icones::finder(dc, pincel, r, opacidade);
                    }
                    None if itens[i].caminho == apps::LIXEIRA => {
                        crate::icones::lixeira(dc, pincel, r, opacidade, lixeira_cheia);
                    }
                    None => {
                        pincel.SetColor(&crate::tema::tinta(0.18 * opacidade));
                        dc.FillRoundedRectangle(&arredondado(r, t * 0.22), pincel);
                    }
                }
            };

            for (i, &(cx, t, base)) in d.icones.iter().enumerate() {
                if i >= itens.len() {
                    // Janela minimizada: um fundo discreto; a miniatura em si
                    // vem do DWM, por cima.
                    let (sx, sy) = tela(cx, base - t / 2.0);
                    let r = retangulo(sx - t * 0.46, sy - t * 0.46, sx + t * 0.46, sy + t * 0.46);
                    pincel.SetColor(&crate::tema::tinta(0.06));
                    dc.FillRoundedRectangle(&arredondado(r, 6.0), pincel);
                    continue;
                }
                if arrastado.is_some_and(|(k, _, _)| k == i) {
                    continue; // fica o espaço vazio; o ícone vai com o mouse
                }
                let cx = cx + itens[i].desloc.x;
                let opacidade = if pressionado == Some(i) { 0.6 } else { 1.0 };
                desenhar_icone(i, cx, t, base, opacidade);
                if !itens[i].janelas.is_empty() {
                    pincel.SetColor(&crate::tema::tinta(0.85));
                    let (px, py) = tela(cx, chao - 4.0);
                    dc.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: px, y: py },
                            radiusX: 2.0,
                            radiusY: 2.0,
                        },
                        pincel,
                    );
                }
            }

            // Anel do foco do teclado.
            if let Some(&(cx, t, base)) = foco.and_then(|f| d.icones.get(f)) {
                let (sx, sy) = tela(cx, base - t / 2.0);
                let r = retangulo(sx - t / 2.0 - 4.0, sy - t / 2.0 - 4.0, sx + t / 2.0 + 4.0, sy + t / 2.0 + 4.0);
                pincel.SetColor(&crate::tema::destaque_vivo(1.0));
                dc.DrawRoundedRectangle(&arredondado(r, t * 0.26), pincel, 2.5, None);
            }

            if let Some((k, x, y)) = arrastado {
                let t = ICONE * 1.15;
                let opacidade = if removendo { 0.45 } else { 0.92 };
                desenhar_icone(k, x, t, y + t / 2.0, opacidade);
            }

            if let Some((layout, cx, topo)) = &rotulo {
                let mut m = DWRITE_TEXT_METRICS::default();
                let _ = layout.GetMetrics(&mut m);
                let pl = m.width + 20.0;
                let ph = m.height + 8.0;
                // Embaixo: o nome fica em cima do ícone. De lado: ao lado dele.
                let (x0, y0) = match posicao {
                    Posicao::Baixo => ((cx - pl / 2.0).clamp(4.0, (larg_dip - pl - 4.0).max(4.0)), (topo - 10.0 - ph).max(2.0)),
                    _ => {
                        let (sx, sy) = tela(*cx, *topo);
                        let x = if posicao == Posicao::Esquerda { sx + 10.0 } else { sx - 10.0 - pl };
                        (x, (sy - ph / 2.0).clamp(2.0, (larg_dip - ph - 2.0).max(2.0)))
                    }
                };
                let caixa = arredondado(retangulo(x0, y0, x0 + pl, y0 + ph), 7.0);
                pincel.SetColor(&crate::tema::fundo(0.11, 0.11, 0.13, 0.88));
                dc.FillRoundedRectangle(&caixa, pincel);
                pincel.SetColor(&crate::tema::tinta(0.14));
                dc.DrawRoundedRectangle(&caixa, pincel, 1.0, None);
                pincel.SetColor(&crate::tema::tinta(0.95));
                dc.DrawTextLayout(
                    D2D_POINT_2F {
                        x: x0 + 10.0,
                        y: y0 + 4.0,
                    },
                    layout,
                    pincel,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        })?;
        self.posicionar_minis(&d);
        QUADROS.fetch_add(1, Ordering::Relaxed);
        self.sujo = false;
        Ok(())
    }

    pub fn largura_dip(&self) -> f32 {
        self.larg_dip
    }

    /// Janelas que estão no dock (para filtrar eventos do Windows).
    pub fn janelas_conhecidas(&self) -> Vec<isize> {
        self.modelo
            .itens
            .iter()
            .flat_map(|i| i.janelas.iter().map(|h| h.0 as isize))
            .collect()
    }
}

/// A janela do dock ocupa a largura toda do monitor principal, encostada na
/// base disponível. O que não é dock é transparente e deixa o clique passar.
/// Sem ocultar automático, o dock reserva a própria altura: janelas
/// maximizadas param em cima dele.
fn posicionar_janela(hwnd: HWND, reservar: bool, auto: bool, ultima: &mut Option<RECT>, posicao: Posicao) {
    let e = janela::escala(hwnd);
    let (mon, trabalho) = janela::monitor_principal();
    let reserva = if auto { 0 } else { ((ALT_FUNDO + DISTANCIA_TELA * 2.0) * e).ceil() as i32 };
    match posicao {
        Posicao::Baixo => {
            let ap = (ALT_JANELA * e).ceil() as i32;
            let base = if reservar {
                janela::reservar_appbar(hwnd, mon, false, reserva, ultima).bottom
            } else {
                trabalho.bottom
            };
            janela::posicionar(hwnd, mon.left, base - ap, mon.right - mon.left, ap);
        }
        Posicao::Esquerda | Posicao::Direita => {
            let lp = (ALT_JANELA_LADO * e).ceil() as i32;
            let direita = posicao == Posicao::Direita;
            let borda = if reservar {
                let r = janela::reservar_appbar_lado(hwnd, mon, direita, reserva, ultima);
                if direita {
                    r.right
                } else {
                    r.left
                }
            } else if direita {
                trabalho.right
            } else {
                trabalho.left
            };
            let x = if direita { borda - lp } else { borda };
            // Fica entre a barra do topo e a base do monitor.
            janela::posicionar(hwnd, x, trabalho.top, lp, trabalho.bottom - trabalho.top);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn coordenadas_vao_e_voltam() {
        for p in [Posicao::Baixo, Posicao::Esquerda, Posicao::Direita] {
            for &(x, y) in &[(0.0, 0.0), (120.5, 40.0), (900.0, 170.0)] {
                let (sx, sy) = para_tela(p, 420.0, x, y);
                let (lx, ly) = para_logico_em(p, 420.0, sx, sy);
                assert!((lx - x).abs() < 1e-3 && (ly - y).abs() < 1e-3, "{p:?} {x},{y}");
            }
        }
    }

    #[test]
    fn borda_da_tela_fica_do_lado_certo() {
        // O "chão" do dock (y grande) encosta na borda de cada lado.
        assert_eq!(para_tela(Posicao::Esquerda, 420.0, 10.0, 420.0).0, 0.0);
        assert_eq!(para_tela(Posicao::Direita, 420.0, 10.0, 420.0).0, 420.0);
        assert_eq!(para_tela(Posicao::Baixo, 176.0, 10.0, 176.0).1, 176.0);
        assert_eq!(Posicao::de("esquerda"), Posicao::Esquerda);
        assert_eq!(Posicao::de("qualquer"), Posicao::Baixo);
    }
}
