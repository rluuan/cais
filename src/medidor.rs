//! Medidor de consumo do próprio Cais (CPU, memória, quadros).
//! Só atualiza enquanto está visível, uma vez por segundo.

use crate::gpu::Gpu;
use crate::janela;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::TIMER_MEDIDOR;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Quadros apresentados pelo dock desde o início.
pub static QUADROS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct Amostra {
    pub instante: Instant,
    pub cpu_100ns: u64,
    pub quadros: u64,
}

pub fn amostrar() -> Amostra {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Amostra {
        instante: Instant::now(),
        cpu_100ns: ft(k) + ft(u),
        quadros: QUADROS.load(Ordering::Relaxed),
    }
}

/// (working set, memória privada) em MB.
pub fn memoria_mb() -> (f64, f64) {
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let mut p = PROCESS_MEMORY_COUNTERS {
        cb,
        ..Default::default()
    };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut p, cb);
    }
    let mb = |b: usize| b as f64 / 1_048_576.0;
    (mb(p.WorkingSetSize), mb(p.PagefileUsage))
}

/// Devolve ao Windows as páginas de memória que não estão em uso agora
/// (bibliotecas de inicialização, ícones já enviados à GPU etc.). O que for
/// preciso depois volta sozinho.
pub fn enxugar_memoria() {
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

pub fn nucleos() -> f64 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f64
}

/// % de CPU da máquina inteira (mesma conta do Gerenciador de Tarefas).
pub fn cpu_pct(antes: &Amostra, depois: &Amostra) -> f64 {
    let seg = (depois.instante - antes.instante).as_secs_f64().max(1e-6);
    let cpu_seg = (depois.cpu_100ns - antes.cpu_100ns) as f64 / 10_000_000.0;
    cpu_seg / seg / nucleos() * 100.0
}

const LARGURA: f32 = 270.0;
const ALTURA: f32 = 138.0;

pub struct Medidor {
    sup: Superficie,
    formato: IDWriteTextFormat,
    pincel: ID2D1SolidColorBrush,
    anterior: Amostra,
    linhas: Vec<String>,
    visivel: bool,
    pub sujo: bool,
}

impl Medidor {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let hwnd = janela::criar(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
        )?;
        posicionar_janela(hwnd);
        let sup = Superficie::new(gpu, hwnd)?;
        unsafe {
            let formato = gpu.dwrite.CreateTextFormat(
                w!("Consolas"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.5,
                crate::idioma::localidade(),
            )?;
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&cor(1.0, 1.0, 1.0, 1.0), None)?;
            Ok(Self {
                sup,
                formato,
                pincel,
                anterior: amostrar(),
                linhas: Vec::new(),
                visivel: false,
                sujo: false,
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn alternar(&mut self, animando: bool) {
        self.visivel = !self.visivel;
        unsafe {
            if self.visivel {
                self.anterior = amostrar();
                self.atualizar(animando);
                let _ = ShowWindow(self.sup.hwnd, SW_SHOWNOACTIVATE);
                SetTimer(self.sup.hwnd, TIMER_MEDIDOR, 1000, None);
            } else {
                let _ = KillTimer(self.sup.hwnd, TIMER_MEDIDOR);
                let _ = ShowWindow(self.sup.hwnd, SW_HIDE);
                self.sujo = false;
            }
        }
    }

    pub fn reposicionar(&mut self, gpu: &Gpu) -> Result<()> {
        posicionar_janela(self.sup.hwnd);
        self.sup.redimensionar(gpu)?;
        self.sujo = self.visivel;
        Ok(())
    }

    pub fn atualizar(&mut self, animando: bool) {
        let agora = amostrar();
        let pct = cpu_pct(&self.anterior, &agora);
        let quadros = agora.quadros - self.anterior.quadros;
        let (ws, privada) = memoria_mb();
        let br = |s: String| s.replace('.', ",");
        self.linhas = vec![
            "Cais, medidor (clique do meio fecha)".into(),
            br(format!("CPU do processo   {pct:>7.3} %")),
            br(format!("Memória em uso    {ws:>7.1} MB")),
            br(format!("Memória privada   {privada:>7.1} MB")),
            format!("Quadros (1 s)     {quadros:>7}"),
            format!("Quadros totais    {:>7}", agora.quadros),
            format!(
                "Estado            {:>7}",
                if animando { "animando" } else { "parado" }
            ),
        ];
        self.anterior = agora;
        self.sujo = true;
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let formato = &self.formato;
        let pincel = &self.pincel;
        let texto: Vec<u16> = self.linhas.join("\n").encode_utf16().collect();
        self.sup.desenhar(gpu, |dc| unsafe {
            pincel.SetColor(&cor(0.07, 0.07, 0.09, 0.86));
            let fundo = arredondado(retangulo(0.5, 0.5, LARGURA - 0.5, ALTURA - 0.5), 12.0);
            dc.FillRoundedRectangle(&fundo, pincel);
            pincel.SetColor(&cor(1.0, 1.0, 1.0, 0.14));
            dc.DrawRoundedRectangle(&fundo, pincel, 1.0, None);
            pincel.SetColor(&cor(0.93, 0.95, 1.0, 0.95));
            dc.DrawText(
                &texto,
                formato,
                &retangulo(14.0, 11.0, LARGURA - 10.0, ALTURA - 8.0),
                pincel,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        })?;
        self.sujo = false;
        Ok(())
    }
}

fn posicionar_janela(hwnd: HWND) {
    let e = janela::escala(hwnd);
    let lp = (LARGURA * e).ceil() as i32;
    let ap = (ALTURA * e).ceil() as i32;
    let area = janela::area_de_trabalho();
    let m = (12.0 * e).round() as i32;
    janela::posicionar(hwnd, area.right - lp - m, area.top + m, lp, ap);
}
