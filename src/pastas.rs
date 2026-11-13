//! "Abrir pastas no Arquivos do Cais": quando o Explorador do Windows abre uma
//! janela nova de pasta comum (ou "Este Computador" / "Início"), o Cais abre
//! o mesmo lugar no Finder e fecha a janela do Windows. Não mexe no
//! registro: desligou, voltou ao normal. Lixeira, Rede, Painel de Controle e
//! outros lugares especiais continuam no Explorador.
//!
//! Segurar Shift enquanto a pasta abre deixa ela no Explorador.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static LIGADO: AtomicBool = AtomicBool::new(false);
static AVISAR: AtomicIsize = AtomicIsize::new(0);
static PASSAR_ATE: Mutex<Option<Instant>> = Mutex::new(None);
/// Resultados prontos: (janela do Explorador, destino).
static PRONTOS: Mutex<Vec<(isize, Destino)>> = Mutex::new(Vec::new());

/// Para onde a janela do Explorador estava indo.
#[derive(Clone, Debug)]
pub enum Destino {
    Pasta(PathBuf),
    Computador,
    /// "Início" / "Acesso rápido": a pasta inicial do Finder.
    Inicio,
}

pub fn ligar(sim: bool) {
    LIGADO.store(sim, Ordering::Relaxed);
}

/// Janela que recebe o aviso `msg` quando uma pasta foi lida.
pub fn definir_aviso(h: HWND, msg: u32) {
    AVISAR.store(h.0 as isize, Ordering::Relaxed);
    MENSAGEM.store(msg, Ordering::Relaxed);
}

static MENSAGEM: AtomicU32 = AtomicU32::new(0);

/// O próprio Cais ia abrir o Explorador ou uma pasta (Explorador no dock,
/// nos Apps, na busca, pasta do dock): abre direto no Finder, sem a janela do
/// Windows aparecer e sumir. Devolve false quando é para seguir no Windows.
pub fn trocar_pelo_finder(caminho: &str) -> bool {
    if !LIGADO.load(Ordering::Relaxed) || unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0 {
        return false;
    }
    if PASSAR_ATE.lock().is_ok_and(|g| g.is_some_and(|t| Instant::now() < t)) {
        return false;
    }
    let minusculo = caminho.trim().trim_matches('"').to_lowercase();
    let destino = if minusculo == "explorer.exe" || minusculo.ends_with(r"\explorer.exe") {
        Destino::Inicio
    } else if minusculo == ESTE_COMPUTADOR.to_lowercase() {
        Destino::Computador
    } else if std::path::Path::new(caminho).is_dir() {
        Destino::Pasta(PathBuf::from(caminho))
    } else {
        return false;
    };
    let aviso = AVISAR.load(Ordering::Relaxed);
    if aviso == 0 {
        return false;
    }
    if let Ok(mut v) = PRONTOS.lock() {
        // Sem janela do Explorador para fechar (0).
        v.push((0, destino));
    }
    unsafe {
        let _ = PostMessageW(HWND(aviso as *mut _), MENSAGEM.load(Ordering::Relaxed), WPARAM(0), LPARAM(0));
    }
    true
}

/// O próprio Cais vai abrir uma janela do Explorador (Lixeira, "Mostrar no
/// Explorador"): a próxima que aparecer nos 3 segundos seguintes fica.
pub fn deixar_passar() {
    if let Ok(mut g) = PASSAR_ATE.lock() {
        *g = Some(Instant::now() + Duration::from_secs(3));
    }
}

fn classe(h: HWND) -> String {
    let mut b = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut b) } as usize;
    String::from_utf16_lossy(&b[..n])
}

/// Chamado no gancho de eventos quando uma janela aparece. Se for uma janela
/// nova do Explorador, lê a pasta dela numa thread e avisa a App com `msg`.
pub fn janela_apareceu(h: HWND, msg: u32) {
    if !LIGADO.load(Ordering::Relaxed) || classe(h) != "CabinetWClass" {
        return;
    }
    unsafe {
        if !GetParent(h).unwrap_or_default().0.is_null() {
            return;
        }
        // Shift segurado: fica no Explorador.
        if GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 {
            return;
        }
    }
    if let Ok(mut g) = PASSAR_ATE.lock() {
        if g.is_some_and(|t| Instant::now() < t) {
            *g = None;
            return;
        }
    }
    let alvo = h.0 as isize;
    let aviso = AVISAR.load(Ordering::Relaxed);
    let _ = std::thread::Builder::new().name("cais-pastas".into()).stack_size(256 * 1024).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // A janela aparece antes de saber a pasta: tenta por até 2 s.
        let fim = Instant::now() + Duration::from_millis(2000);
        let mut achou = None;
        while Instant::now() < fim {
            if let Some(d) = destino_da_janela(HWND(alvo as *mut _)) {
                achou = Some(d);
                break;
            }
            std::thread::sleep(Duration::from_millis(60));
        }
        unsafe {
            CoUninitialize();
        }
        if let Some(d) = achou {
            if let Ok(mut v) = PRONTOS.lock() {
                v.push((alvo, d));
            }
            unsafe {
                let _ = PostMessageW(HWND(aviso as *mut _), msg, WPARAM(0), LPARAM(0));
            }
        }
    });
}

/// Pastas lidas desde o último aviso.
pub fn prontos() -> Vec<(HWND, Destino)> {
    PRONTOS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default().into_iter().map(|(h, d)| (HWND(h as *mut _), d)).collect()
}

/// Fecha a janela do Explorador que foi trocada pelo Finder.
pub fn fechar_explorador(h: HWND) {
    unsafe {
        let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

const ESTE_COMPUTADOR: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
const INICIO: &str = "::{F874310E-B6B7-47DC-BC84-B9E6B38F5903}";
const ACESSO_RAPIDO: &str = "::{679F85CB-0220-4080-B29B-5540CC05AAB6}";

/// Lê, pela lista de janelas do Shell, para onde a janela `h` está aberta.
/// None = ainda não sabe, ou é um lugar especial (fica no Explorador).
fn destino_da_janela(h: HWND) -> Option<Destino> {
    unsafe {
        let janelas: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let n = janelas.Count().ok()?;
        for i in 0..n {
            let Ok(disp) = janelas.Item(&VARIANT::from(i)) else { continue };
            let Ok(nav) = disp.cast::<IWebBrowser2>() else { continue };
            let Ok(hw) = nav.HWND() else { continue };
            if hw.0 as isize != h.0 as isize {
                continue;
            }
            let sp: IServiceProvider = nav.cast().ok()?;
            let navegador: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
            let vista = navegador.QueryActiveShellView().ok()?;
            let fv: IFolderView = vista.cast().ok()?;
            let pasta: IPersistFolder2 = fv.GetFolder().ok()?;
            let pidl: *mut ITEMIDLIST = pasta.GetCurFolder().ok()?;
            let r = destino_do_pidl(pidl);
            CoTaskMemFree(Some(pidl as _));
            return r;
        }
        None
    }
}

unsafe fn destino_do_pidl(pidl: *mut ITEMIDLIST) -> Option<Destino> {
    let mut buf = [0u16; 260];
    if SHGetPathFromIDListW(pidl, &mut buf).as_bool() {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        if n > 0 {
            let p = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
            // Pastas de sistema que o Finder não mostra bem ficam no Windows.
            return p.is_dir().then_some(Destino::Pasta(p));
        }
    }
    let nome = SHGetNameFromIDList(pidl, SIGDN_DESKTOPABSOLUTEPARSING).ok()?;
    let s = nome.to_string().unwrap_or_default();
    CoTaskMemFree(Some(nome.0 as _));
    let s = s.to_uppercase();
    if s == ESTE_COMPUTADOR {
        Some(Destino::Computador)
    } else if s == INICIO || s == ACESSO_RAPIDO {
        Some(Destino::Inicio)
    } else {
        None
    }
}
