//! Tema do Cais: aparência (automática, escura ou clara), cor de destaque e
//! transparência do vidro. Todas as telas pegam as cores daqui; ao mudar,
//! o vidro é retingido na hora e a App pede um quadro novo de cada tela.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::UI::Color;
use windows::UI::Composition::CompositionColorBrush;

/// Aparência: 0 = automática (segue o Windows), 1 = escura, 2 = clara.
pub const APARENCIAS: [(&str, &str); 3] = [("automatico", "Automática"), ("escuro", "Escura"), ("claro", "Clara")];

/// Cores de destaque (as mesmas opções do Mac) e a do Windows.
pub const DESTAQUES: [(&str, &str, (f32, f32, f32)); 9] = [
    ("azul", "Azul", (0.04, 0.52, 1.0)),
    ("roxo", "Roxo", (0.69, 0.32, 0.87)),
    ("rosa", "Rosa", (1.0, 0.18, 0.42)),
    ("vermelho", "Vermelho", (1.0, 0.27, 0.23)),
    ("laranja", "Laranja", (1.0, 0.58, 0.0)),
    ("amarelo", "Amarelo", (1.0, 0.78, 0.0)),
    ("verde", "Verde", (0.16, 0.76, 0.3)),
    ("grafite", "Grafite", (0.56, 0.56, 0.6)),
    ("windows", "A do Windows", (0.04, 0.52, 1.0)),
];

/// Transparência do vidro (multiplica a opacidade da tinta).
pub const TRANSPARENCIAS: [(&str, &str, f32); 3] = [("pouca", "Pouca", 1.7), ("media", "Média", 1.0), ("muita", "Muita", 0.55)];

static MODO: AtomicU8 = AtomicU8::new(0);
static DESTAQUE: AtomicU8 = AtomicU8::new(0);
static TRANSP: AtomicU8 = AtomicU8::new(1);
static SISTEMA_ESCURO: AtomicBool = AtomicBool::new(true);
/// Alto contraste do Windows ligado: vidro opaco e texto mais forte.
static ALTO_CONTRASTE: AtomicBool = AtomicBool::new(false);

pub fn alto_contraste() -> bool {
    ALTO_CONTRASTE.load(Ordering::Relaxed)
}
/// Destaque do Windows (RGB 0-255 empacotado), lido do registro.
static DESTAQUE_WINDOWS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x0A84FF);

/// Tipo de vidro (cada tela tem a sua tinta).
#[derive(Clone, Copy, PartialEq)]
pub enum Tinta {
    Dock,
    Topo,
    Painel,
    Launchpad,
}

struct Pincel {
    id: u64,
    tipo: Tinta,
    pincel: CompositionColorBrush,
}

thread_local! {
    static PINCEIS: RefCell<Vec<Pincel>> = const { RefCell::new(Vec::new()) };
    static PROXIMO: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

pub fn indice(lista: &[&str], nome: &str) -> usize {
    lista.iter().position(|n| *n == nome).unwrap_or(0)
}

/// Lê as escolhas (nomes do config) e retinge o vidro. Devolve se mudou.
pub fn configurar(aparencia: &str, destaque: &str, transparencia: &str) -> bool {
    let m = APARENCIAS.iter().position(|a| a.0 == aparencia).unwrap_or(0) as u8;
    let d = DESTAQUES.iter().position(|a| a.0 == destaque).unwrap_or(0) as u8;
    let t = TRANSPARENCIAS.iter().position(|a| a.0 == transparencia).unwrap_or(1) as u8;
    let antes = (MODO.load(Ordering::Relaxed), DESTAQUE.load(Ordering::Relaxed), TRANSP.load(Ordering::Relaxed), escuro());
    MODO.store(m, Ordering::Relaxed);
    DESTAQUE.store(d, Ordering::Relaxed);
    TRANSP.store(t, Ordering::Relaxed);
    ler_sistema();
    let mudou = antes != (m, d, t, escuro());
    retingir();
    mudou
}

/// O Windows mudou de claro/escuro ou de cor de destaque (WM_SETTINGCHANGE).
/// Devolve se o Cais precisa se redesenhar.
pub fn sistema_mudou() -> bool {
    let antes = (escuro(), destaque(), alto_contraste());
    ler_sistema();
    let mudou = antes != (escuro(), destaque(), alto_contraste());
    if mudou {
        retingir();
    }
    mudou
}

fn ler_dword(chave: windows::core::PCWSTR, valor: windows::core::PCWSTR) -> Option<u32> {
    use windows::Win32::System::Registry::*;
    let mut dado = 0u32;
    let mut tam = 4u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, chave, valor, RRF_RT_REG_DWORD, None, Some(&mut dado as *mut _ as _), Some(&mut tam))
    };
    r.is_ok().then_some(dado)
}

fn ler_sistema() {
    use windows::core::w;
    let claro = ler_dword(w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("AppsUseLightTheme"));
    SISTEMA_ESCURO.store(claro.map_or(true, |v| v == 0), Ordering::Relaxed);
    ALTO_CONTRASTE.store(crate::janela::alto_contraste(), Ordering::Relaxed);
    // AccentColor é 0xAABBGGRR.
    if let Some(c) = ler_dword(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor")) {
        let (r, g, b) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
        DESTAQUE_WINDOWS.store((r << 16) | (g << 8) | b, Ordering::Relaxed);
    }
}

pub fn escuro() -> bool {
    match MODO.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => SISTEMA_ESCURO.load(Ordering::Relaxed),
    }
}

/// Cor de destaque (RGB 0-1).
pub fn destaque() -> (f32, f32, f32) {
    let d = DESTAQUE.load(Ordering::Relaxed) as usize;
    if DESTAQUES.get(d).is_some_and(|x| x.0 == "windows") {
        let c = DESTAQUE_WINDOWS.load(Ordering::Relaxed);
        return (((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0);
    }
    DESTAQUES.get(d).map_or(DESTAQUES[0].2, |x| x.2)
}

/// Destaque mais vivo para traços finos sobre o fundo (cursor de texto,
/// ícones da lateral, contornos).
pub fn destaque_vivo(a: f32) -> D2D1_COLOR_F {
    let (r, g, b) = destaque();
    if escuro() {
        D2D1_COLOR_F { r: r + (1.0 - r) * 0.3, g: g + (1.0 - g) * 0.3, b: b + (1.0 - b) * 0.3, a }
    } else {
        D2D1_COLOR_F { r: r * 0.92, g: g * 0.92, b: b * 0.92, a }
    }
}

/// Cor do destaque com opacidade.
pub fn destaque_a(a: f32) -> D2D1_COLOR_F {
    let (r, g, b) = destaque();
    D2D1_COLOR_F { r, g, b, a }
}

/// Texto e traços por cima do vidro: branco no escuro, quase preto no claro.
/// `a` é a força (a mesma escala dos dois lados).
pub fn tinta(a: f32) -> D2D1_COLOR_F {
    let a = if alto_contraste() { (a * 1.6).min(1.0) } else { a };
    if escuro() {
        D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a }
    } else {
        D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: (a * 0.86).min(0.86) }
    }
}

/// Branco nos dois temas (ícones sobre quadrados coloridos, botão de
/// interruptor).
pub fn branco(a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a }
}

/// Texto que pode estar sobre o destaque (item selecionado): branco em cima
/// do destaque, tinta normal fora dele.
pub fn tinta_sobre(no_destaque: bool, a: f32) -> D2D1_COLOR_F {
    if no_destaque {
        branco(a)
    } else {
        tinta(a)
    }
}

/// Fundo opaco (conteúdo do Finder, balões, menus). Recebe a cor do tema
/// escuro e devolve a equivalente no claro.
pub fn fundo(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    if escuro() {
        D2D1_COLOR_F { r, g, b, a }
    } else {
        let c = |v: f32| (0.985 - v * 0.3).clamp(0.0, 1.0);
        D2D1_COLOR_F { r: c(r), g: c(g), b: c(b), a }
    }
}

/// Tinta do vidro para cada tela.
pub fn cor_vidro(t: Tinta) -> Color {
    let mult = TRANSPARENCIAS.get(TRANSP.load(Ordering::Relaxed) as usize).map_or(1.0, |x| x.2);
    let (a, rgb) = if escuro() {
        match t {
            Tinta::Dock => (84.0, (28, 28, 32)),
            Tinta::Topo => (64.0, (24, 24, 28)),
            Tinta::Painel => (128.0, (30, 30, 34)),
            Tinta::Launchpad => (150.0, (16, 16, 22)),
        }
    } else {
        match t {
            Tinta::Dock => (120.0, (246, 246, 248)),
            Tinta::Topo => (110.0, (246, 246, 248)),
            Tinta::Painel => (170.0, (242, 242, 245)),
            Tinta::Launchpad => (120.0, (236, 236, 240)),
        }
    };
    if alto_contraste() {
        // Sem vidro: fundo sólido, para o texto ficar sempre legível.
        let c = if escuro() { 0 } else { 255 };
        return Color { A: 255, R: c, G: c, B: c };
    }
    Color { A: (a * mult).clamp(0.0, 250.0) as u8, R: rgb.0, G: rgb.1, B: rgb.2 }
}

/// Um vidro novo guarda o pincel da tinta (para ser retingido depois).
pub fn registrar(tipo: Tinta, pincel: CompositionColorBrush) -> u64 {
    let id = PROXIMO.with(|p| {
        let v = p.get();
        p.set(v + 1);
        v
    });
    PINCEIS.with(|l| l.borrow_mut().push(Pincel { id, tipo, pincel }));
    id
}

pub fn esquecer(id: u64) {
    PINCEIS.with(|l| l.borrow_mut().retain(|p| p.id != id));
}

fn retingir() {
    PINCEIS.with(|l| {
        for p in l.borrow().iter() {
            let _ = p.pincel.SetColor(cor_vidro(p.tipo));
        }
    });
}

/// Menus do Windows (clique direito) no mesmo tema.
pub fn modo_dos_menus() -> i32 {
    if escuro() {
        2 // ForceDark
    } else {
        3 // ForceLight
    }
}

#[cfg(test)]
mod testes {
    #[test]
    fn rotulos_em_ingles() {
        let _t = crate::idioma::TRAVA_DE_TESTE.lock().unwrap_or_else(|e| e.into_inner());
        crate::idioma::configurar("en");
        assert_eq!(crate::idioma::rotulo("Escura"), "Dark");
        assert_eq!(crate::idioma::rotulo("Grade 2x2 (4 janelas)"), "2x2 grid (4 windows)");
        crate::idioma::configurar("pt");
        assert_eq!(crate::idioma::rotulo("Escura"), "Escura");
    }
}
