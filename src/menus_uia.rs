//! Menus de apps "modernos" na barra do topo: programas que desenham a
//! própria barra de menus (Bloco de Notas novo, apps WPF, WinForms, WinUI)
//! não têm o menu clássico do Windows, mas o mostram pela Automação de
//! Interface (a mesma que os leitores de tela usam).
//!
//! Tudo roda numa thread à parte (um app travado não trava o Cais), só
//! quando você troca de app, e nunca em navegadores/apps Electron: neles a
//! consulta liga o modo de acessibilidade do Chromium (mais lento) e eles
//! não têm barra de menus mesmo.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Chegaram os títulos de menu de uma janela (wParam = HWND).
pub const WM_MENUS_UIA: u32 = WM_APP + 14;

enum Pedido {
    Ler(isize),
    Abrir(isize, usize),
}

static CANAL: OnceLock<Mutex<Sender<Pedido>>> = OnceLock::new();
static AVISAR: AtomicIsize = AtomicIsize::new(0);
static RESULTADO: Mutex<Option<(isize, Vec<String>)>> = Mutex::new(None);
static ULTIMO: Mutex<(isize, Option<std::time::Instant>)> = Mutex::new((0, None));

fn enviar(p: Pedido) {
    let canal = CANAL.get_or_init(|| {
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new().name("menus".into()).spawn(move || trabalhar(rx));
        Mutex::new(tx)
    });
    if let Ok(tx) = canal.lock() {
        let _ = tx.send(p);
    }
}

/// Ler menus de apps Electron (opção das Preferências, desligada por padrão).
static ELECTRON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn ler_electron(sim: bool) {
    ELECTRON.store(sim, Ordering::Relaxed);
}

/// Classe da janela: navegadores ficam de fora sempre (não têm barra de
/// menus); apps Electron só com a opção ligada.
fn eh_chromium_ou_firefox(h: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    let classe = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    if classe == "MozillaWindowClass" {
        return true;
    }
    if !classe.starts_with("Chrome_WidgetWin") {
        return false;
    }
    if !ELECTRON.load(Ordering::Relaxed) {
        return true;
    }
    let exe = crate::apps::exe_do_pid(crate::apps::pid_do_app(h)).unwrap_or_default().to_lowercase();
    ["\\chrome.exe", "\\msedge.exe", "\\brave.exe", "\\vivaldi.exe", "\\opera.exe", "\\msedgewebview2.exe"]
        .iter()
        .any(|b| exe.ends_with(b))
}

/// Para quem vai o aviso WM_MENUS_UIA (a barra do topo principal).
pub fn definir_aviso(h: HWND) {
    AVISAR.store(h.0 as isize, Ordering::Relaxed);
}

/// Pede os títulos dos menus de `h` (a resposta chega como WM_MENUS_UIA e
/// é lida com `pegar`). Cada barra do topo pede; repetido em seguida vale 1.
pub fn pedir(h: HWND) {
    if h.0.is_null() || eh_chromium_ou_firefox(h) {
        return;
    }
    if let Ok(mut u) = ULTIMO.lock() {
        if u.0 == h.0 as isize && u.1.is_some_and(|t| t.elapsed().as_millis() < 500) {
            return;
        }
        *u = (h.0 as isize, Some(std::time::Instant::now()));
    }
    enviar(Pedido::Ler(h.0 as isize));
}

/// Abre o menu k do app (o próprio app mostra o menu dele).
pub fn abrir(h: HWND, k: usize) {
    unsafe {
        let _ = SetForegroundWindow(h);
    }
    enviar(Pedido::Abrir(h.0 as isize, k));
}

pub fn pegar() -> Option<(HWND, Vec<String>)> {
    RESULTADO.lock().ok()?.take().map(|(h, v)| (HWND(h as *mut _), v))
}

struct Leitor {
    auto: IUIAutomation,
    /// Última janela lida e os itens da barra de menus dela.
    janela: isize,
    itens: Vec<IUIAutomationElement>,
}

fn trabalhar(rx: Receiver<Pedido>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(auto) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else { return };
        // Um app travado não segura a thread por muito tempo.
        if let Ok(a2) = auto.cast::<IUIAutomation2>() {
            let _ = a2.SetConnectionTimeout(1500);
            let _ = a2.SetTransactionTimeout(1500);
        }
        let mut l = Leitor { auto, janela: 0, itens: Vec::new() };
        while let Ok(mut p) = rx.recv() {
            // Trocou de app várias vezes seguidas: só a última importa.
            while let Ok(prox) = rx.try_recv() {
                p = match (p, prox) {
                    (_, Pedido::Ler(h)) => Pedido::Ler(h),
                    (_, a) => a,
                };
            }
            match p {
                Pedido::Ler(h) => {
                    let titulos = l.ler(h);
                    if let Ok(mut r) = RESULTADO.lock() {
                        *r = Some((h, titulos));
                    }
                    let avisar = HWND(AVISAR.load(Ordering::Relaxed) as *mut _);
                    let _ = PostMessageW(avisar, WM_MENUS_UIA, WPARAM(h as usize), LPARAM(0));
                }
                Pedido::Abrir(h, k) => {
                    if l.janela != h {
                        l.ler(h);
                    }
                    if let Some(item) = l.itens.get(k) {
                        if let Ok(p) = item.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
                            let _ = p.Expand();
                        } else if let Ok(p) = item.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                            let _ = p.Invoke();
                        }
                    }
                }
            }
        }
    }
}

impl Leitor {
    unsafe fn ler(&mut self, h: isize) -> Vec<String> {
        self.janela = h;
        self.itens.clear();
        let Ok(raiz) = self.auto.ElementFromHandle(HWND(h as *mut _)) else { return Vec::new() };
        let Ok(cond) = self.auto.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_MenuBarControlTypeId.0)) else {
            return Vec::new();
        };
        let Ok(barras) = raiz.FindAll(TreeScope_Descendants, &cond) else { return Vec::new() };
        let n = barras.Length().unwrap_or(0);
        for i in 0..n.min(6) {
            let Ok(barra) = barras.GetElement(i) else { continue };
            // A "barra" do menu de sistema (mover, fechar...) não conta.
            let id = barra.CurrentAutomationId().map(|b| b.to_string()).unwrap_or_default();
            if id.eq_ignore_ascii_case("SystemMenuBar") {
                continue;
            }
            let Ok(todos) = self.auto.CreateTrueCondition() else { continue };
            let Ok(filhos) = barra.FindAll(TreeScope_Children, &todos) else { continue };
            let mut titulos = Vec::new();
            let mut itens = Vec::new();
            for j in 0..filhos.Length().unwrap_or(0).min(12) {
                let Ok(item) = filhos.GetElement(j) else { continue };
                let nome = item.CurrentName().map(|b| b.to_string()).unwrap_or_default().replace('&', "");
                let nome = nome.trim().to_string();
                if nome.is_empty() {
                    continue;
                }
                titulos.push(nome);
                itens.push(item);
            }
            // Só "Sistema" = o menu de sistema da barra de título.
            let so_sistema = titulos.len() == 1 && matches!(titulos[0].to_lowercase().as_str(), "sistema" | "system");
            if !titulos.is_empty() && !so_sistema {
                self.itens = itens;
                return titulos;
            }
        }
        Vec::new()
    }
}

/// cais.exe --testar-menus: só lê (não clica) os menus de cada janela aberta.
pub fn testar() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(auto) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else {
            println!("menus: sem UI Automation");
            return;
        };
        let mut l = Leitor { auto, janela: 0, itens: Vec::new() };
        for (h, _) in crate::apps::enumerar_janelas() {
            let mut buf = [0u16; 80];
            let n = GetWindowTextW(h, &mut buf) as usize;
            let titulo = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
            if !GetMenu(h).0.is_null() {
                println!("{titulo:40} -> menu clássico");
                continue;
            }
            if eh_chromium_ou_firefox(h) {
                println!("{titulo:40} -> (navegador/Electron, pulado)");
                continue;
            }
            let t0 = std::time::Instant::now();
            let v = l.ler(h.0 as isize);
            println!("{titulo:40} -> {:?} em {} ms", v, t0.elapsed().as_millis());
        }
    }
}