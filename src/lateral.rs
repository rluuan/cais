//! Faixa lateral no estilo do Stage Manager do Mac: janela minimizada vira
//! uma telinha ao vivo, inclinada em perspectiva, na borda esquerda da tela,
//! com o ícone do app no canto. Um clique traz a janela de volta.
//!
//! A telinha é desenhada pelo compositor do Windows (DWM thumbnail): nada é
//! copiado pela CPU, e parada ela não custa nada ao Cais. O Windows não
//! deixa inclinar uma miniatura, então cada telinha é feita de fatias
//! verticais finas, cada uma esticada na altura que a perspectiva pede.

use crate::apps;
use crate::gpu::Gpu;
use crate::janela;
use crate::genio;
use crate::teste3d;
use crate::mola::Mola;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use windows::core::{w, Interface, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Largura da faixa (DIPs): é o espaço reservado na esquerda.
const LARGURA: f32 = 330.0;
const MAX_L: f32 = 206.0;
/// Altura de todos os cartões (DIPs).
const ALTO: f32 = 128.0;
const ESPACO: f32 = 44.0;
const FATIAS: usize = 24;
/// Quanto a borda de trás encolhe (0,16 = 16% mais baixa que a da frente).
const FUGA: f32 = 0.12;
const WM_MOUSELEAVE: u32 = 0x02A3;

// 3D de verdade (camada do compositor): tamanho do cartão "de frente",
// ângulo e distância da câmera, como no Stage Manager.
/// Todos os cartões têm este tamanho (antes de girar), como na referência.
const PLANO_L: f32 = 284.0;
const PLANO_A: f32 = 196.0;
/// Cantos arredondados do cartão (DIPs).
const RAIO_CARTAO: f32 = 12.0;
const ANGULO: f32 = 33.0;
const CAMERA: f32 = 860.0;
/// Altura (fração) do eixo da perspectiva: perto da base, então o topo
/// do lado de trás cai mais que a base sobe.
const PIVO: f32 = 0.92;

/// DirectComposition da faixa: a miniatura vira uma camada que aceita
/// transformação 3D. Fica por baixo dos ícones (que são do Superficie).
struct Comp3d {
    dev: IDCompositionDesktopDevice,
    _alvo: IDCompositionTarget,
    raiz: IDCompositionVisual2,
    criar: teste3d::CriarVisual,
}

fn criar_comp3d(gpu: &Gpu, hwnd: HWND) -> Option<Comp3d> {
    let criar = teste3d::funcao()?;
    unsafe {
        let dev: IDCompositionDesktopDevice = DCompositionCreateDevice3(&gpu.dxgi_device).ok()?;
        // topmost = falso: fica abaixo da camada do Superficie (ícones).
        let alvo = dev.CreateTargetForHwnd(hwnd, false).ok()?;
        let raiz = dev.CreateVisual().ok()?;
        alvo.SetRoot(&raiz).ok()?;
        Some(Comp3d { dev, _alvo: alvo, raiz, criar })
    }
}

#[derive(Clone, Copy)]
pub struct OpcoesLateral {
    /// Telinhas inclinadas em 3D (falso = retas).
    pub inclinar: bool,
    /// Reservar a faixa da esquerda (janelas maximizadas não cobrem).
    pub reservar: bool,
    pub maximo: usize,
    /// Tamanho das telinhas: 1,0 = grande (padrão), menor = menor.
    pub escala: f32,
    /// Efeito gênio ao minimizar e restaurar.
    pub genio: bool,
}

struct Cartao {
    hwnd: HWND,
    /// Uma miniatura do DWM por fatia (só quando o 3D de verdade falha).
    fatias: Vec<isize>,
    /// Miniatura como camada do compositor (3D de verdade) e o id dela.
    camada: Option<(IDCompositionVisual2, isize, IDCompositionRectangleClip)>,
    /// Tamanho da parte visível da janela (sem a borda invisível).
    origem: (f32, f32),
    /// Borda invisível de redimensionar (esq, topo, dir, base), em px da
    /// janela: o Windows 11 a inclui na miniatura, transparente.
    corte: (f32, f32, f32, f32),
    titulo: String,
    exe: String,
    icone: Option<ID2D1Bitmap1>,
    /// 0 = fora da tela (entrando pela esquerda), 1 = no lugar.
    entrada: Mola,
    /// 0 = normal, 1 = mouse em cima (cresce um pouco).
    realce: Mola,
    /// Transparência extra: 0 enquanto o efeito gênio ainda está chegando.
    aparecer: Mola,
}

/// Contorno de um cartão na tela (DIPs).
#[derive(Clone, Copy)]
struct Forma {
    esq: f32,
    dir: f32,
    /// Topo e base da borda esquerda; a direita é calculada pela perspectiva.
    topo: f32,
    altura: f32,
    /// c da perspectiva (0 = reto).
    c: f32,
    icone: D2D_RECT_F,
    /// Cartão "de frente" para a camada 3D: (largura, altura, y, escala).
    plano: (f32, f32, f32, f32),
}

impl Forma {
    /// Posição x da fração u (0 = borda esquerda, 1 = direita) da janela.
    fn x(&self, u: f32) -> f32 {
        self.esq + (self.dir - self.esq) * u * (1.0 + self.c) / (1.0 + self.c * u)
    }
    /// (topo, base) da coluna u: mais longe = mais baixa, caindo pelo topo.
    fn coluna(&self, u: f32) -> (f32, f32) {
        let h = self.altura / (1.0 + self.c * u);
        let encolheu = self.altura - h;
        let topo = self.topo + encolheu * 0.88;
        (topo, topo + h)
    }
    fn cantos(&self) -> [(f32, f32); 4] {
        let (t0, b0) = self.coluna(0.0);
        let (t1, b1) = self.coluna(1.0);
        [(self.esq, t0), (self.dir, t1), (self.dir, b1), (self.esq, b0)]
    }
}

pub struct Lateral {
    sup: Superficie,
    pincel: ID2D1SolidColorBrush,
    texto: IDWriteTextFormat,
    cartoes: Vec<Cartao>,
    pub opcoes: OpcoesLateral,
    comp3d: Option<Comp3d>,
    /// Monitor desta faixa (há uma em cada monitor).
    monitor: HMONITOR,
    registrada: bool,
    ultima: Option<RECT>,
    alt: f32,
    hover: Option<usize>,
    pressionado: Option<usize>,
    rastreando: bool,
    visivel: bool,
    tela_cheia: bool,
    /// Clique numa telinha com o gênio ligado: a App anima e restaura.
    pub pedido_volta: Option<(HWND, genio::Ret)>,
    pub sujo: bool,
}

impl Drop for Lateral {
    fn drop(&mut self) {
        for c in &self.cartoes {
            soltar(c, self.comp3d.as_ref());
        }
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

fn cheia() -> Mola {
    let mut m = Mola::default();
    m.x = 1.0;
    m.alvo = 1.0;
    m
}

/// Último monitor visto de cada janela (hwnd -> HMONITOR), anotado enquanto
/// ela está na tela.
static MONITOR_VISTO: std::sync::Mutex<Option<std::collections::HashMap<isize, isize>>> = std::sync::Mutex::new(None);

/// Anota o monitor onde a janela está agora (fim de arrasto, foco,
/// restaurou). Minimizada, não anota: o Windows já não sabe responder.
pub fn anotar_monitor(h: HWND) {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL};
    unsafe {
        if IsIconic(h).as_bool() {
            return;
        }
        let m = MonitorFromWindow(h, MONITOR_DEFAULTTONULL);
        if m.is_invalid() {
            return;
        }
        if let Ok(mut g) = MONITOR_VISTO.lock() {
            g.get_or_insert_with(Default::default).insert(h.0 as isize, m.0 as isize);
        }
    }
}

/// A janela fechou: esquece o monitor dela.
pub fn esquecer_monitor(h: HWND) {
    if let Ok(mut g) = MONITOR_VISTO.lock() {
        if let Some(m) = g.as_mut() {
            m.remove(&(h.0 as isize));
        }
    }
}

/// Monitor de uma janela. Minimizada, usa o último monitor visto: o Windows
/// responde pelo tamanho "solto" dela, que numa janela maximizada ou
/// encaixada levada a outro monitor ainda aponta para o monitor antigo.
pub fn monitor_da_janela(h: HWND) -> HMONITOR {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    unsafe {
        if IsIconic(h).as_bool() {
            let visto = MONITOR_VISTO.lock().ok().and_then(|g| g.as_ref()?.get(&(h.0 as isize)).copied());
            if let Some(m) = visto {
                let m = HMONITOR(m as _);
                // O monitor ainda existe? (pode ter sido desligado)
                let mut info = windows::Win32::Graphics::Gdi::MONITORINFO {
                    cbSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32,
                    ..Default::default()
                };
                if windows::Win32::Graphics::Gdi::GetMonitorInfoW(m, &mut info).as_bool() {
                    return m;
                }
            }
        }
        MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY)
    }
}

/// Borda invisível que o Windows 11 põe em volta de janelas redimensionáveis
/// (esquerda, direita e base; o topo é a barra de título, que aparece).
/// É a soma da borda de redimensionar com o "padding", menos 1 px visível.
fn borda_invisivel(h: HWND) -> (f32, f32, f32, f32) {
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    unsafe {
        let estilo = GetWindowLongW(h, GWL_STYLE) as u32;
        if estilo & WS_THICKFRAME.0 == 0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let dpi = GetDpiForWindow(h).max(96);
        let b = (GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) - 1).max(0) as f32;
        (b, 0.0, b, b)
    }
}

/// Esconde já a miniatura do cartão (sem esperar o próximo quadro): ela é
/// ao vivo e mostraria o app se redesenhando ao ser restaurado.
fn apagar_miniatura(c: &Cartao) {
    let props = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
        opacity: 0,
        fVisible: false.into(),
        ..Default::default()
    };
    unsafe {
        for &f in &c.fatias {
            let _ = DwmUpdateThumbnailProperties(f, &props);
        }
        if let Some((_, id, _)) = &c.camada {
            let _ = DwmUpdateThumbnailProperties(*id, &props);
        }
    }
}

fn soltar(c: &Cartao, comp: Option<&Comp3d>) {
    for &f in &c.fatias {
        unsafe {
            let _ = DwmUnregisterThumbnail(f);
        }
    }
    if let Some((v, id, _)) = &c.camada {
        unsafe {
            if let Some(comp) = comp {
                let _ = comp.raiz.RemoveVisual(v);
                let _ = comp.dev.Commit();
            }
            let _ = DwmUnregisterThumbnail(*id);
        }
    }
}

impl Lateral {
    pub fn new(gpu: &Gpu, registrar: bool, opcoes: OpcoesLateral, monitor: HMONITOR) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)?;
        if registrar {
            janela::registrar_appbar(hwnd, crate::dock::WM_APPBAR);
        }
        janela::posicionar(hwnd, 0, 0, LARGURA as i32, 400);
        let sup = Superficie::new(gpu, hwnd)?;
        let comp3d = criar_comp3d(gpu, hwnd);
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let texto = gpu.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                crate::idioma::localidade(),
            )?;
            texto.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            texto.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            texto.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sinal = gpu.dwrite.CreateEllipsisTrimmingSign(&texto)?;
            texto.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                },
                &sinal,
            )?;
            Ok(Self {
                sup,
                pincel,
                texto,
                cartoes: Vec::new(),
                opcoes,
                comp3d,
                monitor,
                registrada: registrar,
                ultima: None,
                alt: 400.0,
                hover: None,
                pressionado: None,
                rastreando: false,
                visivel: false,
                tela_cheia: false,
                pedido_volta: None,
                sujo: false,
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    /// Largura da faixa (DIPs), acompanha o tamanho das telinhas.
    fn largura(&self) -> f32 {
        LARGURA * self.opcoes.escala.clamp(0.5, 1.2)
    }

    pub fn monitor(&self) -> HMONITOR {
        self.monitor
    }

    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    /// Opções mudaram (pela tela de Preferências).
    pub fn definir_opcoes(&mut self, gpu: &Gpu, o: OpcoesLateral) {
        self.opcoes = o;
        while self.cartoes.len() > o.maximo.max(1) {
            let c = self.cartoes.remove(0);
            soltar(&c, self.comp3d.as_ref());
        }
        self.reposicionar(gpu);
    }

    /// Reserva (ou devolve) a faixa da esquerda e mostra/esconde a janela.
    pub fn reposicionar(&mut self, gpu: &Gpu) {
        let e = janela::escala(self.sup.hwnd);
        let tem = !self.cartoes.is_empty();
        let larg_px = (self.largura() * e).round() as i32;
        let (mon, _) = janela::info_monitor(self.monitor);
        let esquerda = if self.registrada {
            let reserva = if tem && self.opcoes.reservar { larg_px } else { 0 };
            janela::reservar_appbar_esquerda(self.sup.hwnd, mon, reserva, &mut self.ultima);
            mon.left
        } else {
            mon.left
        };
        let (_, trab) = janela::info_monitor(self.monitor);
        let altura = (trab.bottom - trab.top).max(100);
        if tem {
            janela::posicionar(self.sup.hwnd, esquerda, trab.top, larg_px, altura);
            let _ = self.sup.redimensionar(gpu);
            self.alt = altura as f32 / self.escala();
        }
        let mostrar = tem && !self.tela_cheia;
        if mostrar != self.visivel {
            self.visivel = mostrar;
            unsafe {
                let _ = ShowWindow(self.sup.hwnd, if mostrar { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
        self.atualizar_regiao();
        self.sujo = true;
    }

    pub fn definir_tela_cheia(&mut self, gpu: &Gpu, cheia: bool) {
        self.tela_cheia = cheia;
        self.reposicionar(gpu);
    }

    pub fn registrar_de_novo(&mut self, gpu: &Gpu) {
        if self.registrada {
            janela::registrar_appbar(self.sup.hwnd, crate::dock::WM_APPBAR);
            self.ultima = None;
        }
        self.reposicionar(gpu);
    }

    pub fn sair_do_windows(&self) {
        if self.registrada {
            janela::remover_appbar(self.sup.hwnd);
        }
    }

    // ------------------------------------------------------------------

    pub fn minimizou(&mut self, gpu: &Gpu, h: HWND) {
        if self.cartoes.iter().any(|c| c.hwnd == h) || !apps::eh_janela_de_app(h, true) {
            return;
        }
        // Primeiro tenta a camada 3D de verdade; se este Windows não deixar,
        // volta para as fatias.
        let camada = self.criar_camada(h);
        let mut fatias = Vec::new();
        if camada.is_none() {
            for _ in 0..FATIAS {
                match unsafe { DwmRegisterThumbnail(self.sup.hwnd, h) } {
                    Ok(f) => fatias.push(f),
                    Err(_) => break,
                }
            }
            if fatias.is_empty() {
                return;
            }
        }
        let id = camada.as_ref().map(|c| c.1).unwrap_or_else(|| fatias[0]);
        let total = unsafe { DwmQueryThumbnailSourceSize(id) }
            .map(|s| (s.cx.max(1) as f32, s.cy.max(1) as f32))
            .unwrap_or((16.0, 10.0));
        let corte = borda_invisivel(h);
        let origem = (
            (total.0 - corte.0 - corte.2).max(16.0),
            (total.1 - corte.1 - corte.3).max(10.0),
        );
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
        let exe = apps::exe_do_pid(apps::pid_do_app(h)).unwrap_or_default();
        let mut entrada = Mola::default();
        entrada.alvo = 1.0;
        self.cartoes.push(Cartao {
            hwnd: h,
            fatias,
            camada,
            origem,
            corte,
            titulo: String::from_utf16_lossy(&buf[..n.min(buf.len())]),
            icone: None,
            exe,
            entrada,
            realce: Mola::default(),
            aparecer: cheia(),
        });
        // Só as mais recentes ficam; as antigas saem da faixa.
        while self.cartoes.len() > self.opcoes.maximo.max(1) {
            let c = self.cartoes.remove(0);
            soltar(&c, self.comp3d.as_ref());
        }
        self.hover = None;
        self.reposicionar(gpu);
    }

    /// Onde fica a telinha da janela h, já no lugar (px da tela).
    pub fn destino(&self, h: HWND) -> Option<genio::Ret> {
        let n = self.cartoes.len();
        let i = self.cartoes.iter().position(|c| c.hwnd == h)?;
        let f = self.dispor()[n - 1 - i];
        let e = self.escala();
        let mut r = RECT::default();
        unsafe {
            GetWindowRect(self.sup.hwnd, &mut r).ok()?;
        }
        let (x, y) = (r.left as f32, r.top as f32);
        Some(genio::Ret { esq: x + f.esq * e, topo: y + f.topo * e, dir: x + f.dir * e, base: y + (f.topo + f.altura) * e })
    }

    /// A janela h acabou de minimizar e o gênio vai trazê-la: a telinha já
    /// fica no lugar, mas invisível. Devolve para onde o gênio deve ir.
    pub fn esperar_genio(&mut self, h: HWND) -> Option<genio::Ret> {
        let i = self.cartoes.iter().position(|c| c.hwnd == h)?;
        let c = &mut self.cartoes[i];
        c.entrada.x = 1.0;
        c.entrada.alvo = 1.0;
        c.aparecer = Mola::default();
        self.sujo = true;
        self.atualizar_regiao();
        self.destino(h)
    }

    /// O gênio chegou (ou desistiu): a telinha aparece.
    pub fn revelar(&mut self, h: HWND) {
        if let Some(c) = self.cartoes.iter_mut().find(|c| c.hwnd == h) {
            c.aparecer.alvo = 1.0;
            self.sujo = true;
        }
    }

    /// Miniatura da janela como camada do compositor, presa na raiz 3D.
    fn criar_camada(&self, h: HWND) -> Option<(IDCompositionVisual2, isize, IDCompositionRectangleClip)> {
        let comp = self.comp3d.as_ref()?;
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY | teste3d::DWM_TNP_ENABLE3D,
            rcDestination: RECT { left: 0, top: 0, right: 16, bottom: 10 },
            rcSource: RECT::default(),
            opacity: 0,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        let mut visual: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut id: isize = 0;
        unsafe {
            let hr = (comp.criar)(self.sup.hwnd, h, 2, &props, comp.dev.as_raw(), &mut visual, &mut id);
            if hr.is_err() || visual.is_null() {
                if id != 0 {
                    let _ = DwmUnregisterThumbnail(id);
                }
                return None;
            }
            let v = IDCompositionVisual2::from_raw(visual);
            comp.raiz.AddVisual(&v, true, None).ok()?;
            // Recorte com cantos arredondados (gira junto com a janela).
            let recorte = comp.dev.CreateRectangleClip().ok()?;
            v.SetClip(&recorte).ok()?;
            Some((v, id, recorte))
        }
    }

    /// Os ícones dos cartões serão carregados de novo no próximo desenho.
    pub fn esquecer_icones(&mut self) {
        for c in &mut self.cartoes {
            c.icone = None;
        }
        self.sujo = true;
    }

    /// Antes de restaurar a janela h: o cartão some na hora.
    pub fn esconder_cartao(&mut self, h: HWND) -> bool {
        let mut achou = false;
        for c in self.cartoes.iter_mut().filter(|c| c.hwnd == h) {
            apagar_miniatura(c);
            c.aparecer = Mola::default();
            achou = true;
        }
        if achou {
            self.sujo = true;
        }
        achou
    }

    pub fn restaurou(&mut self, gpu: &Gpu, h: HWND) {
        let antes = self.cartoes.len();
        let comp = self.comp3d.as_ref();
        self.cartoes.retain(|c| {
            if c.hwnd == h {
                soltar(c, comp);
                false
            } else {
                true
            }
        });
        if self.cartoes.len() != antes {
            self.hover = None;
            self.pressionado = None;
            self.reposicionar(gpu);
        }
    }

    /// Ao abrir o Cais (ou ao trocar para "Lateral"): pega as já minimizadas.
    pub fn ler_minimizadas(&mut self, gpu: &Gpu) {
        for (h, _) in apps::enumerar_janelas().into_iter().rev() {
            if unsafe { IsIconic(h).as_bool() } && monitor_da_janela(h) == self.monitor {
                self.minimizou(gpu, h);
            }
        }
    }

    /// Tira as telinhas de janelas que fecharam ou voltaram por fora.
    pub fn conferir(&mut self, gpu: &Gpu) {
        let mortas: Vec<HWND> = self
            .cartoes
            .iter()
            .filter(|c| unsafe { !IsWindow(c.hwnd).as_bool() || !IsIconic(c.hwnd).as_bool() })
            .map(|c| c.hwnd)
            .collect();
        for h in mortas {
            self.restaurou(gpu, h);
        }
    }

    pub fn limpar(&mut self, gpu: &Gpu) {
        let todas: Vec<HWND> = self.cartoes.iter().map(|c| c.hwnd).collect();
        for h in todas {
            self.restaurou(gpu, h);
        }
    }

    fn trazer(&mut self, gpu: &Gpu, i: usize) {
        let h = self.cartoes[i].hwnd;
        if self.opcoes.genio && self.pedido_volta.is_none() {
            // Com o gênio: a telinha some e a animação leva a janela de
            // volta; quem restaura de verdade é a App, no fim dela.
            if let Some(r) = self.destino(h) {
                self.cartoes[i].aparecer = Mola::default();
                self.sujo = true;
                self.pedido_volta = Some((h, r));
                return;
            }
        }
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        self.restaurou(gpu, h);
    }

    // ------------------------------------------------------------------
    // Disposição (DIPs). A mais recente fica em cima.

    fn dispor(&self) -> Vec<Forma> {
        let n = self.cartoes.len();
        let c_persp = if self.opcoes.inclinar { FUGA / (1.0 - FUGA) } else { 0.0 };
        // Largura na tela: a perspectiva também encurta a largura.
        let encurta = if self.opcoes.inclinar { 0.80 } else { 1.0 };
        // Todos os cartões do mesmo tamanho: a janela preenche o cartão e o
        // que sobra do formato dela é cortado (ver `recorte`).
        // Cada janela no formato dela (inteira, sem cortar), todas alinhadas
        // pela borda esquerda.
        let tamanhos: Vec<(f32, f32)> = self
            .cartoes
            .iter()
            .rev()
            .map(|c| {
                let (w, h) = c.origem;
                if c.camada.is_some() {
                    // Todos do mesmo tamanho; a janela preenche (ver posicionar).
                    (PLANO_L * self.opcoes.escala, PLANO_A * self.opcoes.escala)
                } else {
                    let k = (MAX_L / (w * encurta)).min(ALTO / h);
                    (w * encurta * k, h * k)
                }
            })
            .collect();
        // Muitas janelas: todas encolhem na mesma proporção (cartões e
        // espaços) para a pilha caber na altura, sem nada cortado.
        let total: f32 = tamanhos.iter().map(|t| t.1).sum::<f32>() + ESPACO * n.saturating_sub(1) as f32;
        let livre = (self.alt - 48.0).max(100.0);
        let f = if total > livre { (livre / total).max(0.35) } else { 1.0 };
        let tamanhos: Vec<(f32, f32)> = tamanhos.into_iter().map(|(l, a)| (l * f, a * f)).collect();
        let espaco = ESPACO * f;
        let total = total * f;
        let mut y = ((self.alt - total) / 2.0).max(24.0);
        let mut v = Vec::with_capacity(n);
        for (k, (l, a)) in tamanhos.into_iter().enumerate() {
            let c = &self.cartoes[n - 1 - k];
            let s = 1.0 + 0.07 * c.realce.x;
            if c.camada.is_some() {
                // 3D de verdade: a borda esquerda fica no lugar (perto da
                // câmera) e a direita se afasta. Cresce em volta do pivô.
                // Espaço à esquerda para o ícone, que sai para fora do canto.
                let lado_base = (a * 0.3).clamp(26.0, 60.0);
                let ox = 14.0 + lado_base * 0.4 - (1.0 - c.entrada.x) * (self.largura() * 0.95);
                let graus = if self.opcoes.inclinar { ANGULO } else { 0.0 };
                let (sen, cos) = graus.to_radians().sin_cos();
                let lw = l * s;
                let largura_vista = lw * cos / (1.0 + lw * sen / CAMERA);
                let pivo = a * PIVO;
                let topo = y + pivo - pivo * s;
                let altura = a * s;
                let lado = (altura * 0.3).clamp(26.0, 60.0);
                let icone = retangulo(ox - lado * 0.4, topo + altura - lado * 0.7, ox + lado * 0.6, topo + altura + lado * 0.3);
                v.push(Forma {
                    esq: ox,
                    dir: ox + largura_vista,
                    topo,
                    altura,
                    c: 0.0,
                    icone,
                    plano: (l, a, y, s),
                });
                y += a + espaco;
                continue;
            }
            let (ls, as_) = (l * s, a * s);
            let esq0 = (self.largura() - MAX_L) / 2.0 + 8.0;
            let centro_y = y + a / 2.0;
            let esq = esq0 - (ls - l) / 2.0 - (1.0 - c.entrada.x) * (self.largura() * 0.95);
            let topo = centro_y - as_ / 2.0;
            let lado = (as_ * 0.3).clamp(22.0, 56.0);
            let icone = retangulo(esq - lado * 0.28, topo + as_ - lado * 0.62, esq - lado * 0.28 + lado, topo + as_ - lado * 0.62 + lado);
            v.push(Forma {
                esq,
                dir: esq + ls,
                topo,
                altura: as_,
                c: c_persp,
                icone,
                plano: (l, a, y, s),
            });
            y += a + espaco;
        }
        v
    }

    /// Índice do cartão (em `cartoes`) sob o ponto.
    fn acertar(&self, x: f32, y: f32) -> Option<usize> {
        let n = self.cartoes.len();
        self.dispor().iter().enumerate().find_map(|(k, f)| {
            let dentro_card = x >= f.esq - 6.0 && x <= f.dir + 6.0 && y >= f.topo - 6.0 && y <= f.topo + f.altura + 6.0;
            let i = &f.icone;
            let dentro_icone = x >= i.left && x <= i.right && y >= i.top && y <= i.bottom;
            (dentro_card || dentro_icone).then_some(n - 1 - k)
        })
    }

    fn atualizar_regiao(&self) {
        let e = self.escala();
        let rs: Vec<RECT> = self
            .dispor()
            .iter()
            .map(|f| RECT {
                left: ((f.esq.min(f.icone.left) - 12.0) * e).floor() as i32,
                top: ((f.topo - 12.0) * e).floor() as i32,
                right: ((f.dir + 12.0) * e).ceil() as i32,
                bottom: ((f.icone.bottom.max(f.topo + f.altura) + 30.0) * e).ceil() as i32,
            })
            .collect();
        janela::definir_regiao_varias(self.sup.hwnd, &rs);
    }

    pub fn animando(&self) -> bool {
        self.cartoes.iter().any(|c| !c.entrada.parada() || !c.realce.parada() || !c.aparecer.parada())
    }

    pub fn passo(&mut self, dt: f32) {
        if !self.animando() {
            return;
        }
        for c in &mut self.cartoes {
            c.entrada.passo(dt);
            c.realce.passo(dt);
            c.aparecer.passo(dt);
        }
        self.sujo = true;
        if !self.animando() {
            self.atualizar_regiao();
        }
    }

    fn definir_hover(&mut self, novo: Option<usize>) {
        if novo == self.hover {
            return;
        }
        self.hover = novo;
        for (i, c) in self.cartoes.iter_mut().enumerate() {
            c.realce.alvo = if Some(i) == novo { 1.0 } else { 0.0 };
        }
        self.sujo = true;
    }

    pub fn mensagem(&mut self, gpu: &Gpu, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(3)), // MA_NOACTIVATE
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
                let alvo = self.acertar(x, y);
                self.definir_hover(alvo);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.rastreando = false;
                self.definir_hover(None);
                self.pressionado = None;
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                self.pressionado = self.acertar(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let alvo = self.acertar(x, y);
                if let Some(i) = alvo.filter(|_| alvo == self.pressionado.take()) {
                    self.trazer(gpu, i);
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                if let Some(i) = self.acertar(x, y) {
                    self.menu(gpu, i);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn menu(&mut self, gpu: &Gpu, i: usize) {
        let cmd = unsafe {
            let Ok(m) = CreatePopupMenu() else { return };
            let restaurar = crate::idioma::largo(crate::idioma::t("Restaurar", "Restore"));
            let fechar = crate::idioma::largo(crate::idioma::t("Fechar janela", "Close window"));
            let todas = crate::idioma::largo(crate::idioma::t("Restaurar todas", "Restore all"));
            let _ = AppendMenuW(m, MF_STRING, 1, PCWSTR(restaurar.as_ptr()));
            let _ = AppendMenuW(m, MF_STRING, 2, PCWSTR(fechar.as_ptr()));
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(m, MF_STRING, 3, PCWSTR(todas.as_ptr()));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.sup.hwnd);
            let c = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, self.sup.hwnd, None).0;
            let _ = DestroyMenu(m);
            c
        };
        if i >= self.cartoes.len() {
            return;
        }
        match cmd {
            1 => self.trazer(gpu, i),
            2 => unsafe {
                let _ = PostMessageW(self.cartoes[i].hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            3 => {
                let todas: Vec<HWND> = self.cartoes.iter().map(|c| c.hwnd).collect();
                for h in todas {
                    unsafe {
                        let _ = ShowWindow(h, SW_RESTORE);
                    }
                }
                self.limpar(gpu);
            }
            _ => {}
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        if !self.visivel {
            self.sujo = false;
            return Ok(());
        }
        let px = (56.0 * self.escala()).round() as u32;
        for c in self.cartoes.iter_mut().filter(|c| c.icone.is_none() && !c.exe.is_empty()) {
            c.icone = if apps::eh_hospedeiro(&c.exe) {
                apps::icone_da_janela(&gpu.d2d_dc, c.hwnd, px).or_else(|| apps::carregar_icone(&gpu.d2d_dc, &c.exe, px))
            } else {
                apps::carregar_icone(&gpu.d2d_dc, &c.exe, px)
            };
        }
        let formas = self.dispor();
        let n = self.cartoes.len();
        // Nome da janela só para a telinha sob o mouse.
        let nome = match self.hover {
            Some(i) if i < n => {
                let w: Vec<u16> = self.cartoes[i].titulo.encode_utf16().collect();
                let f = formas[n - 1 - i];
                Some((unsafe { gpu.dwrite.CreateTextLayout(&w, &self.texto, (f.dir - f.esq).max(60.0), 24.0)? }, f))
            }
            _ => None,
        };
        let fabrica = unsafe { gpu.d2d_dc.GetFactory()? };
        let p = &self.pincel;
        let cartoes = &self.cartoes;
        self.sup.desenhar(gpu, |dc| unsafe {
            for (k, f) in formas.iter().enumerate() {
                // Contorno quase invisível, só no modo de fatias (a camada
                // 3D já tem a borda da própria janela).
                let c = &cartoes[n - 1 - k];
                if c.camada.is_some() {
                    continue;
                }
                let op = c.entrada.x.clamp(0.0, 1.0);
                if let Ok(g) = fabrica.CreatePathGeometry() {
                    if let Ok(s) = g.Open() {
                        let cs = f.cantos();
                        s.BeginFigure(D2D_POINT_2F { x: cs[0].0, y: cs[0].1 }, D2D1_FIGURE_BEGIN_FILLED);
                        for &(x, y) in &cs[1..] {
                            s.AddLine(D2D_POINT_2F { x, y });
                        }
                        s.EndFigure(D2D1_FIGURE_END_CLOSED);
                        if s.Close().is_ok() {
                            // Só um contorno quase invisível (sem placa).
                            p.SetColor(&cor(0.0, 0.0, 0.0, 0.12 * op));
                            dc.DrawGeometry(&g, p, 1.0, None);
                        }
                    }
                }
            }
            // Ícone do app no canto de baixo à esquerda, por cima de tudo.
            for (k, f) in formas.iter().enumerate() {
                let c = &cartoes[n - 1 - k];
                let op = c.entrada.x.clamp(0.0, 1.0) * c.aparecer.x.clamp(0.0, 1.0);
                let r = f.icone;
                match &c.icone {
                    Some(b) => {
                        p.SetColor(&cor(0.0, 0.0, 0.0, 0.25 * op));
                        dc.FillRoundedRectangle(
                            &arredondado(retangulo(r.left + 1.0, r.top + 3.0, r.right + 1.0, r.bottom + 3.0), (r.right - r.left) * 0.24),
                            p,
                        );
                        dc.DrawBitmap(b, Some(&r), op, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
                    }
                    None => {
                        p.SetColor(&cor(0.3, 0.3, 0.34, op));
                        dc.FillRoundedRectangle(&arredondado(r, (r.right - r.left) * 0.24), p);
                    }
                }
            }
            if let Some((l, f)) = &nome {
                let y = f.topo + f.altura + 8.0;
                let mut m = DWRITE_TEXT_METRICS::default();
                let _ = l.GetMetrics(&mut m);
                let cx = (f.esq + f.dir) / 2.0;
                let w = m.width + 20.0;
                p.SetColor(&crate::tema::fundo(0.08, 0.08, 0.1, 0.82));
                dc.FillRoundedRectangle(&arredondado(retangulo(cx - w / 2.0, y, cx + w / 2.0, y + 22.0), 11.0), p);
                p.SetColor(&crate::tema::tinta(0.95));
                dc.DrawTextLayout(D2D_POINT_2F { x: cx - l.GetMaxWidth() / 2.0, y: y - 1.0 }, l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
        })?;
        self.posicionar_fatias(&formas);
        self.sujo = false;
        Ok(())
    }

    /// Onde a janela inteira cabe dentro da placa, sem cortar nem ampliar
    /// além do necessário: (u0, u1, v0, v1) em frações da placa.
    fn encaixe(&self, _c: &Cartao) -> (f32, f32, f32, f32) {
        // O cartão já tem o formato da janela: ela ocupa tudo.
        (0.0, 1.0, 0.0, 1.0)
    }

    #[allow(dead_code)]
    fn encaixe_em_placa(&self, c: &Cartao) -> (f32, f32, f32, f32) {
        let encurta = if self.opcoes.inclinar { 0.80 } else { 1.0 };
        let margem = 0.05;
        let alvo = (MAX_L / encurta) / ALTO;
        let (w, h) = c.origem;
        let livre = 1.0 - 2.0 * margem;
        if w / h > alvo {
            let vh = livre * alvo / (w / h);
            (margem, 1.0 - margem, (1.0 - vh) / 2.0, (1.0 + vh) / 2.0)
        } else {
            let uw = livre * (w / h) / alvo;
            ((1.0 - uw) / 2.0, (1.0 + uw) / 2.0, margem, 1.0 - margem)
        }
    }

    /// Cada fatia da janela vai para a coluna dela no trapézio. Perto do
    /// ícone as fatias param antes, para o ícone ficar por cima.
    fn posicionar_fatias(&self, formas: &[Forma]) {
        let e = self.escala();
        let n = self.cartoes.len();
        let mut mudou_3d = false;
        for (k, f) in formas.iter().enumerate() {
            let c = &self.cartoes[n - 1 - k];
            if let Some((v, id, recorte)) = &c.camada {
                let (l, a, y, s) = f.plano;
                // A janela preenche o cartão: corta as sobras do formato dela
                // (as laterais das largas, a base das altas; o topo, com a
                // barra de título, sempre aparece).
                let (vw, vh) = c.origem;
                let alvo = l / a;
                let (rx, sw, sh) = if vw / vh > alvo { ((vw - vh * alvo) / 2.0, vh * alvo, vh) } else { (0.0, vw, vw / alvo) };
                let (cl, ct) = (c.corte.0 + rx, c.corte.1);
                let opac = c.entrada.x.clamp(0.0, 1.0) * c.aparecer.x.clamp(0.0, 1.0);
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | teste3d::DWM_TNP_ENABLE3D,
                    rcDestination: RECT { left: 0, top: 0, right: (l * e).round() as i32, bottom: (a * e).round() as i32 },
                    rcSource: RECT {
                        left: cl as i32,
                        top: ct as i32,
                        right: (cl + sw).round() as i32,
                        bottom: (ct + sh).round() as i32,
                    },
                    opacity: (opac * 255.0) as u8,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                let graus = if self.opcoes.inclinar { ANGULO } else { 0.0 };
                let pv = a * PIVO * e;
                // Cresce (hover) em volta do pivô e depois gira com perspectiva.
                let ida = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -pv, 0.0, 1.0];
                let escala = [s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
                let volta = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, pv, 0.0, 1.0];
                let m = teste3d::mult(
                    &teste3d::mult(&teste3d::mult(&ida, &escala), &volta),
                    &teste3d::inclinar(0.0, pv, graus, CAMERA * e),
                );
                unsafe {
                    let _ = DwmUpdateThumbnailProperties(*id, &props);
                    let r = RAIO_CARTAO * e * f.plano.3.min(1.0).max(0.6);
                    let _ = recorte.SetLeft2(0.0);
                    let _ = recorte.SetTop2(0.0);
                    let _ = recorte.SetRight2((l * e).round());
                    let _ = recorte.SetBottom2((a * e).round());
                    let _ = recorte.SetTopLeftRadiusX2(r);
                    let _ = recorte.SetTopLeftRadiusY2(r);
                    let _ = recorte.SetTopRightRadiusX2(r);
                    let _ = recorte.SetTopRightRadiusY2(r);
                    let _ = recorte.SetBottomLeftRadiusX2(r);
                    let _ = recorte.SetBottomLeftRadiusY2(r);
                    let _ = recorte.SetBottomRightRadiusX2(r);
                    let _ = recorte.SetBottomRightRadiusY2(r);
                    if let Ok(v3) = v.cast::<IDCompositionVisual3>() {
                        let _ = v3.SetOffsetX2((f.esq * e).round());
                        let _ = v3.SetOffsetY2((y * e).round());
                        let _ = v3.SetTransform2(&teste3d::matriz(m));
                    }
                }
                mudou_3d = true;
                continue;
            }
            // A janela inteira, centralizada dentro da placa.
            let (sw, sh) = c.origem;
            let (cl, ct) = (c.corte.0, c.corte.1);
            let (ua, ub, va, vb) = self.encaixe(c);
            let total = c.fatias.len().max(1);
            let opac = (c.entrada.x.clamp(0.0, 1.0) * c.aparecer.x.clamp(0.0, 1.0) * 255.0) as u8;
            for (j, &fatia) in c.fatias.iter().enumerate() {
                // t = fração da janela; u = fração da placa.
                let t0 = j as f32 / total as f32;
                let t1 = (j + 1) as f32 / total as f32;
                let u0 = ua + (ub - ua) * t0;
                let u1 = ua + (ub - ua) * t1;
                let (topo_placa, base_placa) = f.coluna((u0 + u1) / 2.0);
                let alt_placa = base_placa - topo_placa;
                let topo = topo_placa + alt_placa * va;
                let base = topo_placa + alt_placa * vb;
                let x0 = f.x(u0);
                let x1 = f.x(u1);
                let mut base_corte = base;
                let mut fonte_base = sh;
                if x0 < f.icone.right - 2.0 && f.icone.top < base {
                    base_corte = (f.icone.top + 4.0).max(topo + 1.0);
                    fonte_base = sh * ((base_corte - topo) / (base - topo)).clamp(0.0, 1.0);
                }
                let (u0, u1) = (t0, t1);
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                    rcDestination: RECT {
                        left: (x0 * e).floor() as i32,
                        top: (topo * e).round() as i32,
                        // +1 px: as fatias se encostam sem frestas.
                        right: (x1 * e).ceil() as i32 + 1,
                        bottom: (base_corte * e).round() as i32,
                    },
                    rcSource: RECT {
                        left: (cl + sw * u0).floor() as i32,
                        top: ct as i32,
                        right: (cl + sw * u1).ceil() as i32,
                        bottom: (ct + fonte_base).round().max(ct + 1.0) as i32,
                    },
                    opacity: opac,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                unsafe {
                    let _ = DwmUpdateThumbnailProperties(fatia, &props);
                }
            }
        }
        if mudou_3d {
            if let Some(comp) = &self.comp3d {
                unsafe {
                    let _ = comp.dev.Commit();
                }
            }
        }
    }
}
