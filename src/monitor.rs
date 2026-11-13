//! Monitor do sistema: CPU, memória, disco, tempo ligado e os apps que mais
//! usam memória. Só lê alguma coisa quando você pediu para ver: na barra
//! (opção, a cada 2 s) ou com o painel aberto (a cada 1 s). Fechado e com a
//! opção desligada, não existe timer nenhum.

use crate::apps;
use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use std::collections::{HashMap, VecDeque};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::SystemInformation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Timer do monitor (na janela da barra do topo principal).
pub const TIMER_MONITOR: usize = 6;
const AMOSTRAS: usize = 60;
const LARGURA: f32 = 320.0;
const P: f32 = 14.0;

#[derive(Clone, Default)]
pub struct Leitura {
    pub cpu: f32,
    pub mem_pct: f32,
    pub mem_usada_gb: f64,
    pub mem_total_gb: f64,
    /// Temperatura da placa de vídeo mais quente, em °C (None: o driver não informa).
    pub gpu_temp: Option<f32>,
}

#[derive(Default)]
pub struct Leitor {
    anterior: Option<(u64, u64)>,
    /// Placas de vídeo abertas para ler a temperatura (handles do D3DKMT).
    placas: Option<Vec<u32>>,
}

// Temperatura da GPU pelo mesmo caminho do Gerenciador de Tarefas
// (D3DKMT, gdi32): vale para NVIDIA, AMD e Intel, sem administrador e sem
// biblioteca do fabricante.
#[repr(C)]
struct InfoDaPlaca {
    h: u32,
    luid: [u32; 2],
    fontes: u32,
    preciso: i32,
}
#[repr(C)]
struct ListaDePlacas {
    n: u32,
    p: *mut InfoDaPlaca,
}
#[repr(C)]
struct Consulta {
    h: u32,
    tipo: i32,
    dados: *mut core::ffi::c_void,
    tam: u32,
}
#[repr(C)]
#[derive(Default)]
struct Desempenho {
    indice: u32,
    mem_freq: u64,
    mem_max: u64,
    mem_max_oc: u64,
    mem_banda: u64,
    pcie: u64,
    ventoinha: u32,
    energia: u32,
    /// Décimos de grau Celsius.
    temperatura: u32,
    estado: u8,
}
#[repr(C)]
struct Fechar {
    h: u32,
}
const DESEMPENHO_DA_PLACA: i32 = 62; // KMTQAITYPE_ADAPTERPERFDATA

#[link(name = "gdi32")]
extern "system" {
    fn D3DKMTEnumAdapters2(l: *mut ListaDePlacas) -> i32;
    fn D3DKMTQueryAdapterInfo(c: *mut Consulta) -> i32;
    fn D3DKMTCloseAdapter(f: *mut Fechar) -> i32;
}

fn abrir_placas() -> Vec<u32> {
    unsafe {
        let mut l = ListaDePlacas { n: 0, p: std::ptr::null_mut() };
        if D3DKMTEnumAdapters2(&mut l) != 0 || l.n == 0 {
            return Vec::new();
        }
        let mut v: Vec<InfoDaPlaca> = (0..l.n).map(|_| InfoDaPlaca { h: 0, luid: [0; 2], fontes: 0, preciso: 0 }).collect();
        l.p = v.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut l) != 0 {
            return Vec::new();
        }
        v.truncate(l.n as usize);
        v.into_iter().map(|i| i.h).collect()
    }
}

fn fechar_placas(placas: &[u32]) {
    for &h in placas {
        unsafe {
            D3DKMTCloseAdapter(&mut Fechar { h });
        }
    }
}

/// Temperatura de uma placa (None se ela não informa, como a de vídeo básica).
fn temperatura(h: u32) -> Option<f32> {
    let mut d = Desempenho::default();
    let mut c = Consulta { h, tipo: DESEMPENHO_DA_PLACA, dados: &mut d as *mut _ as _, tam: std::mem::size_of::<Desempenho>() as u32 };
    let ok = unsafe { D3DKMTQueryAdapterInfo(&mut c) } == 0;
    (ok && d.temperatura > 0 && d.temperatura < 1500).then(|| d.temperatura as f32 / 10.0)
}

impl Drop for Leitor {
    fn drop(&mut self) {
        if let Some(p) = self.placas.take() {
            fechar_placas(&p);
        }
    }
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Leitor {
    /// A placa mais quente. Abre as placas uma vez; se a leitura falhar
    /// (driver atualizado, placa removida), reabre na próxima.
    fn temperatura_da_gpu(&mut self) -> Option<f32> {
        let placas = self.placas.get_or_insert_with(abrir_placas);
        let t = placas.iter().filter_map(|&h| temperatura(h)).reduce(f32::max);
        if t.is_none() {
            if let Some(p) = self.placas.take() {
                fechar_placas(&p);
            }
        }
        t
    }

    /// CPU do sistema todo (% desde a leitura anterior) e memória.
    pub fn ler(&mut self) -> Leitura {
        let mut cpu = 0.0;
        unsafe {
            let (mut ocioso, mut kernel, mut usuario) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetSystemTimes(Some(&mut ocioso), Some(&mut kernel), Some(&mut usuario)).is_ok() {
                let total = ft(kernel) + ft(usuario); // kernel já inclui o ocioso
                let parado = ft(ocioso);
                if let Some((t0, p0)) = self.anterior {
                    let dt = total.saturating_sub(t0) as f32;
                    if dt > 0.0 {
                        cpu = (1.0 - parado.saturating_sub(p0) as f32 / dt).clamp(0.0, 1.0) * 100.0;
                    }
                }
                self.anterior = Some((total, parado));
            }
        }
        let mut m = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        let _ = unsafe { GlobalMemoryStatusEx(&mut m) };
        let gb = |b: u64| b as f64 / 1024.0 / 1024.0 / 1024.0;
        let gpu_temp = self.temperatura_da_gpu();
        Leitura {
            gpu_temp,
            cpu,
            mem_pct: m.dwMemoryLoad as f32,
            mem_usada_gb: gb(m.ullTotalPhys - m.ullAvailPhys),
            mem_total_gb: gb(m.ullTotalPhys),
        }
    }
}

/// (livre, total) do disco C:, em GB.
fn disco() -> (f64, f64) {
    let (mut livre, mut total) = (0u64, 0u64);
    unsafe {
        let _ = GetDiskFreeSpaceExW(w!("C:\\"), None, Some(&mut total), Some(&mut livre));
    }
    let gb = |b: u64| b as f64 / 1024.0 / 1024.0 / 1024.0;
    (gb(livre), gb(total))
}

/// Os n apps que mais usam memória (somando as janelas/processos de cada um).
fn mais_memoria(n: usize) -> Vec<(String, f64)> {
    let mut pids = vec![0u32; 2048];
    let mut bytes = 0u32;
    unsafe {
        if K32EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes).as_bool() == false {
            return Vec::new();
        }
    }
    pids.truncate(bytes as usize / 4);
    let mut por_app: HashMap<String, f64> = HashMap::new();
    for pid in pids.into_iter().filter(|&p| p > 4) {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let mut c = PROCESS_MEMORY_COUNTERS::default();
            let ok = K32GetProcessMemoryInfo(h, &mut c, std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32).as_bool();
            let _ = CloseHandle(h);
            if !ok {
                continue;
            }
            let Some(exe) = apps::exe_do_pid(pid) else { continue };
            *por_app.entry(exe).or_insert(0.0) += c.WorkingSetSize as f64 / 1024.0 / 1024.0;
        }
    }
    let mut v: Vec<(String, f64)> = por_app.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(n);
    v.into_iter().map(|(exe, mb)| (apps::nome_do_exe(&exe), mb)).collect()
}

/// "3 d 4 h", "5 h 12 min"...
fn tempo_ligado() -> String {
    let s = unsafe { GetTickCount64() } / 1000;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

/// Texto curto para a barra do topo.
pub fn texto_da_barra(l: &Leitura) -> String {
    let mut s = format!("CPU {:.0}%   MEM {:.0}%", l.cpu, l.mem_pct);
    if let Some(t) = l.gpu_temp {
        s += &format!("   GPU {:.0}°", t);
    }
    s
}

fn gb(v: f64) -> String {
    format!("{:.1}", v).replace('.', ",")
}

// ---------------------------------------------------------------------------
// Painel

pub struct Painel {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    titulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    numero: IDWriteTextFormat,
    historico: VecDeque<f32>,
    leitura: Leitura,
    disco: (f64, f64),
    apps: Vec<(String, f64)>,
    altura: f32,
    pub visivel: bool,
    fechado_em: Option<std::time::Instant>,
    contador: u32,
    pub sujo: bool,
}

impl Painel {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW)?;
        janela::preparar_vidro(hwnd);
        janela::posicionar(hwnd, 0, 0, LARGURA as i32, 400);
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 16.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |peso, tam, direita: bool| -> Result<IDWriteTextFormat> {
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
                if direita {
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
                titulo: fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0, false)?,
                texto: fonte(DWRITE_FONT_WEIGHT_NORMAL, 12.5, false)?,
                numero: fonte(DWRITE_FONT_WEIGHT_NORMAL, 12.5, true)?,
                historico: VecDeque::with_capacity(AMOSTRAS),
                leitura: Leitura::default(),
                disco: (0.0, 0.0),
                apps: Vec::new(),
                altura: 400.0,
                visivel: false,
                fechado_em: None,
                contador: 0,
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
        self.fechado_em.map_or(false, |t| t.elapsed().as_millis() < 300)
    }

    pub fn abrir(&mut self, gpu: &Gpu, leitura: Leitura, direita: i32, baixo: i32) {
        self.visivel = true;
        self.historico.clear();
        self.leitura = leitura;
        self.disco = disco();
        self.apps = mais_memoria(5);
        self.contador = 0;
        self.altura = 14.0 + 128.0 + 14.0 + 64.0 + 14.0 + 40.0 + 14.0 + 32.0 + self.apps.len() as f32 * 26.0 + 14.0 + 40.0 + 14.0;
        let e = self.escala();
        let (l, a) = ((LARGURA * e).round() as i32, (self.altura * e).round() as i32);
        janela::posicionar(self.sup.hwnd, direita - l - (8.0 * e) as i32, baixo + (6.0 * e) as i32, l, a);
        let _ = self.sup.redimensionar(gpu);
        let _ = self.vidro.definir(0.0, 0.0, l as f32, a as f32, 16.0 * e);
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
        self.apps.clear();
        self.historico.clear();
    }

    /// Uma leitura nova (timer de 1 s com o painel aberto).
    pub fn atualizar(&mut self, l: Leitura) {
        if self.historico.len() == AMOSTRAS {
            self.historico.pop_front();
        }
        self.historico.push_back(l.cpu);
        self.leitura = l;
        // Lista de apps e disco mudam devagar: a cada 5 s basta.
        self.contador += 1;
        if self.contador % 5 == 0 {
            self.apps = mais_memoria(5);
            self.disco = disco();
        }
        self.sujo = true;
    }

    fn botao(&self) -> D2D_RECT_F {
        retangulo(P, self.altura - 14.0 - 32.0, LARGURA - P, self.altura - 14.0)
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
            WM_LBUTTONUP => {
                let b = self.botao();
                if x >= b.left && x <= b.right && y >= b.top && y <= b.bottom {
                    self.fechar();
                    apps::abrir("taskmgr.exe");
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let l = &self.leitura;
        let (livre, total) = self.disco;
        let mut textos: Vec<(String, D2D_RECT_F, u8, f32)> = Vec::new(); // (texto, ret, fonte, alfa)
        let mut y = 14.0;
        textos.push(("CPU".into(), retangulo(P, y, 160.0, y + 20.0), 0, 0.95));
        textos.push((format!("{:.0}%", l.cpu), retangulo(160.0, y, LARGURA - P, y + 20.0), 2, 0.95));
        let grafico = retangulo(P, y + 26.0, LARGURA - P, y + 118.0);
        y += 128.0 + 14.0;
        textos.push((crate::idioma::t("Memória", "Memory").into(), retangulo(P, y, 160.0, y + 20.0), 0, 0.95));
        textos.push((
            format!("{} de {} GB", gb(l.mem_usada_gb), gb(l.mem_total_gb)),
            retangulo(140.0, y, LARGURA - P, y + 20.0),
            2,
            0.8,
        ));
        let barra_mem = retangulo(P, y + 30.0, LARGURA - P, y + 40.0);
        y += 64.0 + 14.0;
        let usado_disco = if total > 0.0 { ((total - livre) / total) as f32 } else { 0.0 };
        textos.push((crate::idioma::t("Disco C:", "Disk C:").into(), retangulo(P, y, 160.0, y + 20.0), 0, 0.95));
        textos.push((format!("{} {}", gb(livre), crate::idioma::t("GB livres", "GB free")), retangulo(140.0, y, LARGURA - P, y + 20.0), 2, 0.8));
        let barra_disco = retangulo(P, y + 26.0, LARGURA - P, y + 34.0);
        y += 40.0 + 14.0;
        textos.push((crate::idioma::t("Usando mais memória", "Using the most memory").into(), retangulo(P, y, 220.0, y + 20.0), 0, 0.95));
        textos.push((format!("{} {}", crate::idioma::t("ligado há", "up for"), tempo_ligado()), retangulo(150.0, y, LARGURA - P, y + 20.0), 2, 0.5));
        y += 32.0;
        for (nome, mb) in &self.apps {
            textos.push((nome.clone(), retangulo(P, y, 220.0, y + 22.0), 1, 0.85));
            let valor = if *mb >= 1024.0 { format!("{} GB", gb(mb / 1024.0)) } else { format!("{:.0} MB", mb) };
            textos.push((valor, retangulo(200.0, y, LARGURA - P, y + 22.0), 2, 0.65));
            y += 26.0;
        }
        let b = self.botao();
        textos.push((crate::idioma::t("Abrir o Gerenciador de Tarefas", "Open Task Manager").into(), retangulo(b.left + 12.0, b.top, b.right, b.bottom), 1, 0.95));
        let fontes = [&self.titulo, &self.texto, &self.numero];
        let mut layouts = Vec::new();
        for (t, r, f, a) in &textos {
            let w: Vec<u16> = t.encode_utf16().collect();
            let lay = unsafe { gpu.dwrite.CreateTextLayout(&w, fontes[*f as usize], r.right - r.left, r.bottom - r.top)? };
            layouts.push((lay, *r, *a));
        }
        let hist: Vec<f32> = self.historico.iter().copied().collect();
        let (mem, p) = (l.mem_pct / 100.0, &self.pincel);
        let fabrica = unsafe { gpu.d2d_dc.GetFactory()? };
        self.sup.desenhar(gpu, |dc| unsafe {
            // Gráfico da CPU (últimos 60 s, enquanto o painel está aberto).
            p.SetColor(&crate::tema::tinta(0.07));
            dc.FillRoundedRectangle(&arredondado(grafico, 8.0), p);
            if hist.len() >= 2 {
                if let Ok(g) = fabrica.CreatePathGeometry() {
                    if let Ok(s) = g.Open() {
                        let passo = (grafico.right - grafico.left) / (AMOSTRAS - 1) as f32;
                        let x0 = grafico.right - passo * (hist.len() - 1) as f32;
                        let alt = grafico.bottom - grafico.top - 8.0;
                        let ponto = |k: usize| D2D_POINT_2F {
                            x: x0 + passo * k as f32,
                            y: grafico.bottom - 4.0 - alt * hist[k] / 100.0,
                        };
                        s.BeginFigure(D2D_POINT_2F { x: x0, y: grafico.bottom }, D2D1_FIGURE_BEGIN_FILLED);
                        for k in 0..hist.len() {
                            s.AddLine(ponto(k));
                        }
                        s.AddLine(D2D_POINT_2F { x: grafico.right, y: grafico.bottom });
                        s.EndFigure(D2D1_FIGURE_END_CLOSED);
                        if s.Close().is_ok() {
                            p.SetColor(&crate::tema::destaque_a(0.35));
                            dc.FillGeometry(&g, p, None);
                            p.SetColor(&crate::tema::destaque_vivo(0.95));
                            dc.DrawGeometry(&g, p, 1.5, None);
                        }
                    }
                }
            }
            for (r, v, rgb) in [(barra_mem, mem, (0.36f32, 0.86f32, 0.45f32)), (barra_disco, usado_disco, (1.0, 0.7, 0.25))] {
                let raio = (r.bottom - r.top) / 2.0;
                p.SetColor(&crate::tema::tinta(0.12));
                dc.FillRoundedRectangle(&arredondado(r, raio), p);
                p.SetColor(&cor(rgb.0, rgb.1, rgb.2, 0.95));
                let fim = r.left + (r.right - r.left) * v.clamp(0.0, 1.0);
                dc.FillRoundedRectangle(&arredondado(retangulo(r.left, r.top, fim.max(r.left + raio * 2.0), r.bottom), raio), p);
            }
            p.SetColor(&crate::tema::tinta(0.1));
            dc.FillRoundedRectangle(&arredondado(b, 8.0), p);
            for (lay, r, a) in &layouts {
                p.SetColor(&crate::tema::tinta(*a));
                dc.DrawTextLayout(D2D_POINT_2F { x: r.left, y: r.top }, lay, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}

#[cfg(test)]
mod teste_gpu {
    #[test]
    #[ignore]
    fn ler_gpu() {
        let p = super::abrir_placas();
        println!("placas {:?}", p);
        for h in &p { println!("{} -> {:?}", h, super::temperatura(*h)); }
    }
}

