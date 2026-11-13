//! Efeito "gênio" ao minimizar: a janela escorre, como um funil, até a
//! telinha dela na faixa da esquerda (e faz o caminho de volta ao restaurar).
//!
//! Como funciona: uma janela transparente cobre o monitor só durante a
//! animação (~0,45 s), sem pegar clique. Nela, a miniatura ao vivo da janela
//! (desenhada pelo compositor do Windows, nada passa pela CPU) é cortada em
//! colunas finas; cada coluna anda e encolhe na altura conforme o funil.
//! Depois a janela some e o Cais volta a gastar zero.

use crate::gpu::Gpu;
use crate::janela;
use crate::teste3d;
use std::time::Instant;
use windows::core::Interface;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const COLUNAS: usize = 36;
const DURACAO_MINIMIZAR: f32 = 0.46;
const DURACAO_RESTAURAR: f32 = 0.38;
/// Volta: quanto o efeito fica parado sobre a janela que voltou, e quanto
/// leva para sumir depois (ms).
const VOLTA_PARADO_MS: f32 = 60.0;
const VOLTA_SUMIR_MS: f32 = 100.0;
/// Volta: tempo total do efeito por cima da janela (a App tira ele depois).
pub const VOLTA_FIM_MS: u128 = 165;

/// Volta: a miniatura acende nesse tempo (ms) no começo. Esconde o quadro
/// preto que o app mostra logo depois de restaurado.
const VOLTA_ACENDER_MS: f32 = 90.0;

/// Janelas restauradas invisíveis (hwnd, estilo estendido original), para
/// devolver ao normal se o Cais sair ou cair no meio do efeito.
static ESCONDIDAS: std::sync::Mutex<Vec<(isize, isize)>> = std::sync::Mutex::new(Vec::new());

/// Volta: restaura a janela já no começo do efeito, mas transparente. Apps
/// como VS Code e Chrome ficam pretos uns 50 ms ao sair do minimizado; assim
/// eles se redesenham durante a animação e aparecem prontos no fim.
/// Falso se não deu (janela de admin, ou que já usa transparência): aí a
/// App restaura do jeito antigo, no fim do efeito.
pub fn restaurar_invisivel(h: HWND) -> bool {
    unsafe {
        // As janelas do próprio Cais já voltam desenhadas.
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        if ex & WS_EX_LAYERED.0 as isize != 0 {
            return false;
        }
        SetLastError(WIN32_ERROR(0));
        if SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize) == 0 && GetLastError().0 != 0 {
            return false;
        }
        if SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA).is_err() {
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
            return false;
        }
        if let Ok(mut v) = ESCONDIDAS.lock() {
            v.push((h.0 as isize, ex));
        }
        let _ = ShowWindow(h, SW_RESTORE);
        let _ = SetForegroundWindow(h);
        true
    }
}

/// Mostra de novo uma janela restaurada por `restaurar_invisivel`.
pub fn mostrar(h: HWND) {
    let ex = ESCONDIDAS.lock().ok().and_then(|mut v| {
        let i = v.iter().position(|(j, _)| *j == h.0 as isize)?;
        Some(v.remove(i).1)
    });
    if let Some(ex) = ex {
        unsafe { voltar_ao_normal(h, ex) };
    }
}

/// Ao sair (ou cair): nenhuma janela pode ficar invisível.
pub fn mostrar_todas() {
    let v: Vec<(isize, isize)> = ESCONDIDAS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
    for (h, ex) in v {
        unsafe { voltar_ao_normal(HWND(h as _), ex) };
    }
}

unsafe fn voltar_ao_normal(h: HWND, ex: isize) {
    let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
    SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
    let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
}

/// Retângulo na tela (px).
#[derive(Clone, Copy, Debug)]
pub struct Ret {
    pub esq: f32,
    pub topo: f32,
    pub dir: f32,
    pub base: f32,
}

impl Ret {
    pub fn de(r: RECT) -> Self {
        Self { esq: r.left as f32, topo: r.top as f32, dir: r.right as f32, base: r.bottom as f32 }
    }
}

struct Coluna {
    visual: IDCompositionVisual2,
    v3: Option<IDCompositionVisual3>,
    id: isize,
}

/// A janela transparente e o compositor do efeito. Ficam prontos (escondidos)
/// desde a abertura do Cais: criar tudo na hora de minimizar atrasava o
/// efeito e a janela "piscava" (sumia antes de o gênio aparecer).
pub struct Palco {
    janela: HWND,
    dev: IDCompositionDesktopDevice,
    _alvo: IDCompositionTarget,
    raiz: IDCompositionVisual2,
}

impl Palco {
    pub fn new(gpu: &Gpu) -> Option<Self> {
        unsafe {
            let janela = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT).ok()?;
            let montar = || -> Option<(IDCompositionDesktopDevice, IDCompositionTarget, IDCompositionVisual2)> {
                let dev: IDCompositionDesktopDevice = DCompositionCreateDevice3(&gpu.dxgi_device).ok()?;
                let alvo = dev.CreateTargetForHwnd(janela, true).ok()?;
                let raiz = dev.CreateVisual().ok()?;
                alvo.SetRoot(&raiz).ok()?;
                let _ = dev.Commit();
                Some((dev, alvo, raiz))
            };
            match montar() {
                Some((dev, alvo, raiz)) => Some(Self { janela, dev, _alvo: alvo, raiz }),
                None => {
                    let _ = DestroyWindow(janela);
                    None
                }
            }
        }
    }
}

impl Drop for Palco {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.janela);
        }
    }
}

pub struct Genio {
    pub hwnd_fonte: HWND,
    /// Voltando (restaurar): a animação corre ao contrário e, no fim, a
    /// janela de verdade aparece.
    pub volta: bool,
    /// A App já fez a parte dela perto do fim (mostrou a telinha ou trouxe
    /// a janela de volta).
    pub marco: bool,
    /// Quando a janela de verdade voltou (o efeito fica por cima dela e some
    /// aos poucos, até ela estar desenhada).
    pub saiu_em: Option<Instant>,
    /// Volta: a janela já foi restaurada, invisível (ver `restaurar_invisivel`).
    pub escondida: bool,
    palco: Option<Palco>,
    colunas: Vec<Coluna>,
    /// Janela (px da tela) e telinha (px da tela).
    de: Ret,
    para: Ret,
    /// Canto do monitor (a janela da animação começa ali).
    origem: (f32, f32),
    /// Tamanho da miniatura (px), igual ao da janela.
    fonte: (f32, f32),
    inicio: Instant,
    duracao: f32,
}

impl Genio {
    /// Esconde a janela do efeito e tira as miniaturas (o palco fica pronto
    /// para a próxima vez).
    fn limpar(&mut self) {
        let Some(p) = &self.palco else { return };
        unsafe {
            let _ = ShowWindow(p.janela, SW_HIDE);
            for c in self.colunas.drain(..) {
                let _ = p.raiz.RemoveVisual(&c.visual);
                let _ = DwmUnregisterThumbnail(c.id);
            }
            let _ = p.dev.Commit();
        }
    }

    /// Terminou: devolve o palco para ser usado de novo.
    pub fn devolver(mut self) -> Option<Palco> {
        self.limpar();
        self.palco.take()
    }

    /// Fração do tempo já passada (0 a 1).
    pub fn tempo(&self) -> f32 {
        (self.inicio.elapsed().as_secs_f32() / self.duracao).min(1.0)
    }
}

impl Drop for Genio {
    fn drop(&mut self) {
        if self.escondida {
            mostrar(self.hwnd_fonte); // não faz nada se já foi mostrada
        }
        self.limpar();
    }
}

/// Matriz (vetor-linha, 4x4) que leva o quadrado unitário ao quadrilátero
/// (cantos: (0,0), (1,0), (1,1), (0,1)). Fórmula de Heckbert.
fn quadrado_para_quad(q: [(f32, f32); 4]) -> [f32; 16] {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q;
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let (a, b, c, d, e, f, g, h);
    let den = dx1 * dy2 - dx2 * dy1;
    if (dx3.abs() < 1e-4 && dy3.abs() < 1e-4) || den.abs() < 1e-6 {
        a = x1 - x0;
        b = x2 - x1;
        c = x0;
        d = y1 - y0;
        e = y2 - y1;
        f = y0;
        g = 0.0;
        h = 0.0;
    } else {
        g = (dx3 * dy2 - dx2 * dy3) / den;
        h = (dx1 * dy3 - dx3 * dy1) / den;
        a = x1 - x0 + g * x1;
        b = x3 - x0 + h * x3;
        c = x0;
        d = y1 - y0 + g * y1;
        e = y3 - y0 + h * y3;
        f = y0;
    }
    // x' = (a u + b v + c) / (g u + h v + 1); y' = (d u + e v + f) / (...)
    [
        a, d, 0.0, g,
        b, e, 0.0, h,
        0.0, 0.0, 1.0, 0.0,
        c, f, 0.0, 1.0,
    ]
}

fn suave(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mistura(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Quanto uma janela maximizada passa da área livre em cada lado (px):
/// a borda de redimensionar (ou a fixa) mais o "padding" do Windows.
fn borda_maximizada(h: HWND) -> (i32, i32) {
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    unsafe {
        let estilo = GetWindowLongW(h, GWL_STYLE) as u32;
        let dpi = GetDpiForWindow(h).max(96);
        let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        if estilo & WS_THICKFRAME.0 != 0 {
            (GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + pad, GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + pad)
        } else if estilo & WS_CAPTION.0 == WS_CAPTION.0 {
            (GetSystemMetricsForDpi(SM_CXFIXEDFRAME, dpi) + pad, GetSystemMetricsForDpi(SM_CYFIXEDFRAME, dpi) + pad)
        } else {
            (0, 0)
        }
    }
}

/// Onde a janela estava antes de minimizar (px da tela, com a borda
/// invisível, que é o que a miniatura mostra).
pub fn retangulo_da_janela(h: HWND) -> Option<Ret> {
    unsafe {
        if !IsIconic(h).as_bool() {
            let mut r = RECT::default();
            GetWindowRect(h, &mut r).ok()?;
            return Some(Ret::de(r));
        }
        let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        GetWindowPlacement(h, &mut wp).ok()?;
        let m = crate::lateral::monitor_da_janela(h);
        let (mon, trab) = janela::info_monitor(m);
        if wp.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0 {
            // Maximizada: a janela passa da área livre pela borda invisível
            // (a miniatura inclui essa borda; sem ela, o fim do efeito fica
            // deslocado e a janela "pula" ao aparecer).
            let (bx, by) = borda_maximizada(h);
            return Some(Ret {
                esq: (trab.left - bx) as f32,
                topo: (trab.top - by) as f32,
                dir: (trab.right + bx) as f32,
                base: (trab.bottom + by) as f32,
            });
        }
        // "Coordenadas da área de trabalho": deslocadas pelas barras
        // reservadas do monitor principal.
        let (pmon, ptrab) = janela::info_monitor(janela::hmonitor_principal());
        let (dx, dy) = ((ptrab.left - pmon.left) as f32, (ptrab.top - pmon.top) as f32);
        let r = wp.rcNormalPosition;
        let _ = mon;
        Some(Ret { esq: r.left as f32 + dx, topo: r.top as f32 + dy, dir: r.right as f32 + dx, base: r.bottom as f32 + dy })
    }
}

impl Genio {
    /// `de` = janela, `para` = telinha (px da tela). `volta` = restaurar.
    pub fn new(palco: Palco, h: HWND, de: Ret, para: Ret, volta: bool) -> Option<Self> {
        let criar = teste3d::funcao()?;
        // A janela do efeito cobre só o caminho da animação (janela + telinha),
        // nunca o monitor inteiro: uma janela do tamanho da tela aparecendo e
        // sumindo faz o Windows mudar o jeito de compor a tela (a tela pisca).
        let folga = 24.0;
        let caixa = RECT {
            left: (de.esq.min(para.esq) - folga).floor() as i32,
            top: (de.topo.min(para.topo) - folga).floor() as i32,
            right: (de.dir.max(para.dir) + folga).ceil() as i32,
            bottom: (de.base.max(para.base) + folga).ceil() as i32,
        };
        unsafe {
            let janela = palco.janela;
            let _ = SetWindowPos(
                janela,
                HWND_TOPMOST,
                caixa.left,
                caixa.top,
                caixa.right - caixa.left,
                caixa.bottom - caixa.top,
                SWP_NOACTIVATE,
            );
            let mut g = Genio {
                hwnd_fonte: h,
                volta,
                marco: false,
                saiu_em: None,
                escondida: false,
                palco: Some(palco),
                colunas: Vec::new(),
                de,
                para,
                origem: (caixa.left as f32, caixa.top as f32),
                fonte: (1.0, 1.0),
                inicio: Instant::now(),
                duracao: if volta { DURACAO_RESTAURAR } else { DURACAO_MINIMIZAR },
            };
            let (dev, raiz) = {
                let p = g.palco.as_ref()?;
                (p.dev.clone(), p.raiz.clone())
            };
            for _ in 0..COLUNAS {
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | teste3d::DWM_TNP_ENABLE3D,
                    rcDestination: RECT { left: 0, top: 0, right: 16, bottom: 10 },
                    rcSource: RECT::default(),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                let mut visual: *mut core::ffi::c_void = std::ptr::null_mut();
                let mut id: isize = 0;
                let hr = criar(janela, h, 2, &props, dev.as_raw(), &mut visual, &mut id);
                if hr.is_err() || visual.is_null() {
                    if id != 0 {
                        let _ = DwmUnregisterThumbnail(id);
                    }
                    return None; // Drop limpa as colunas já criadas
                }
                let v = IDCompositionVisual2::from_raw(visual);
                let _ = raiz.AddVisual(&v, true, None);
                let v3 = v.cast::<IDCompositionVisual3>().ok();
                g.colunas.push(Coluna { visual: v, v3, id });
            }
            let tam = DwmQueryThumbnailSourceSize(g.colunas[0].id).ok()?;
            g.fonte = (tam.cx.max(1) as f32, tam.cy.max(1) as f32);
            // Cada coluna mostra a janela inteira, recortada na faixa dela.
            let (fw, fh) = g.fonte;
            for (j, c) in g.colunas.iter().enumerate() {
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | teste3d::DWM_TNP_ENABLE3D,
                    rcDestination: RECT { left: 0, top: 0, right: fw as i32, bottom: fh as i32 },
                    rcSource: RECT::default(),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                let _ = DwmUpdateThumbnailProperties(c.id, &props);
                let x0 = (fw * j as f32 / COLUNAS as f32).floor();
                let x1 = (fw * (j + 1) as f32 / COLUNAS as f32).ceil() + 1.0;
                if let Ok(clip) = dev.CreateRectangleClip() {
                    let _ = clip.SetLeft2(x0);
                    let _ = clip.SetRight2(x1.min(fw));
                    let _ = clip.SetTop2(0.0);
                    let _ = clip.SetBottom2(fh);
                    let _ = c.visual.SetClip(&clip);
                }
            }
            g.aplicar(if volta { 1.0 } else { 0.0 });
            // Só aparece com o 1º quadro já pronto no compositor (senão a
            // janela do efeito surge vazia por um instante).
            let _ = dev.WaitForCommitCompletion();
            let _ = ShowWindow(janela, SW_SHOWNOACTIVATE);
            g.inicio = Instant::now();
            Some(g)
        }
    }

    /// Só para o teste: animação mais lenta.
    pub fn devagar(&mut self, seg: f32) {
        self.duracao = seg;
        self.inicio = Instant::now();
    }

    /// Terminou?
    pub fn passo(&mut self) -> bool {
        let t = (self.inicio.elapsed().as_secs_f32() / self.duracao).min(1.0);
        self.aplicar(if self.volta { 1.0 - t } else { t });
        t >= 1.0
    }

    /// Desenha o quadro do progresso p (0 = janela, 1 = telinha).
    fn aplicar(&self, p: f32) {
        let (de, para) = (self.de, self.para);
        let (fw, _fh) = self.fonte;
        let larg_de = de.dir - de.esq;
        let larg_para = para.dir - para.esq;
        // Fase 1: o funil se forma (a altura perto da faixa encolhe).
        let curva = suave(p / 0.45);
        // Funil: x da boca (na telinha) até onde a janela começava.
        let boca = para.dir;
        let fundo = de.esq.max(boca + 80.0);
        let altura_em = |x: f32| -> (f32, f32) {
            let w = 1.0 - suave((x - boca) / (fundo - boca)); // 1 na boca, 0 longe
            let w = w * curva;
            (mistura(de.topo, para.topo, w), mistura(de.base, para.base, w))
        };
        let x_de = |u: f32, s: f32| mistura(de.esq + u * larg_de, para.esq + u * larg_para, s);
        // Fase 2: a janela desliza para o funil; a parte da frente (perto da
        // faixa) vai primeiro e a de trás estica atrás dela. O progresso é
        // calculado nas BORDAS entre colunas: vizinhas dividem a mesma borda
        // e a janela estica sem rasgar.
        let progresso = |u: f32| {
            let atraso = 0.22 + 0.2 * u;
            suave((p - atraso) / (1.0 - atraso).max(0.01))
        };
        // Cada borda: (x, topo, base) na tela, já relativo à janela da animação.
        let bordas: Vec<(f32, f32, f32)> = (0..=COLUNAS)
            .map(|b| {
                let u = b as f32 / COLUNAS as f32;
                let s = progresso(u);
                let x = x_de(u, s);
                let (topo, base) = altura_em(x);
                let (topo, base) = (mistura(topo, para.topo, s * s), mistura(base, para.base, s * s));
                (x - self.origem.0, topo - self.origem.1, base - self.origem.1)
            })
            .collect();
        let fh = self.fonte.1;
        unsafe {
            for (j, c) in self.colunas.iter().enumerate() {
                let x0src = fw * j as f32 / COLUNAS as f32;
                let larg_src = fw / COLUNAS as f32;
                let (xa, ta, ba) = bordas[j];
                let (xb, tb, bb) = bordas[j + 1];
                // A faixa da miniatura vira o quadrilátero entre as duas
                // bordas (topo e base inclinados): sem degraus.
                let h = quadrado_para_quad([(xa, ta), (xb, tb), (xb, bb), (xa, ba)]);
                let s = [
                    1.0 / larg_src, 0.0, 0.0, 0.0,
                    0.0, 1.0 / fh, 0.0, 0.0,
                    0.0, 0.0, 1.0, 0.0,
                    -x0src / larg_src, 0.0, 0.0, 1.0,
                ];
                if let Some(v3) = &c.v3 {
                    let _ = v3.SetTransform2(&teste3d::matriz(teste3d::mult(&s, &h)));
                }
            }
            let Some(palco) = &self.palco else { return };
            // Ida: some nos últimos instantes, enquanto a telinha de verdade
            // aparece por baixo (as duas se cruzam, sem quadro vazio no meio).
            // Volta: opaco desde o 1º quadro (cobre a telinha que sumiu) e,
            // depois que a janela voltou, fica parado um pouco e some aos
            // poucos por cima dela (cobre o app que demora a se desenhar).
            let alfa = if self.volta {
                match self.saiu_em {
                    Some(t) => 1.0 - suave((t.elapsed().as_secs_f32() * 1000.0 - VOLTA_PARADO_MS) / VOLTA_SUMIR_MS),
                    None if self.escondida => suave(self.inicio.elapsed().as_secs_f32() * 1000.0 / VOLTA_ACENDER_MS),
                    None => 1.0,
                }
            } else {
                1.0 - suave((p - 0.86) / 0.14)
            };
            if let Ok(v3) = palco.raiz.cast::<IDCompositionVisual3>() {
                let _ = v3.SetOpacity2(alfa);
            }
            let _ = palco.dev.Commit();
        }
    }
}
