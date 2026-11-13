//! Ícones da bandeja (área perto do relógio) na barra do topo.
//!
//! Os programas avisam "tenho um ícone" mandando uma mensagem para a
//! primeira janela chamada "Shell_TrayWnd" que encontram. O Cais cria uma
//! janela invisível com esse nome, guarda uma cópia do ícone e REPASSA cada
//! mensagem, sem mudar nada, para a bandeja do Explorer. Assim o Windows
//! continua sabendo de tudo: se o Cais fechar ou travar, nada se perde.
//! (É a mesma técnica de barras de tarefas alternativas de código aberto.)
//!
//! Tudo acontece entre janelas do seu PC; nada sai dele.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::core::{w, GUID, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A lista de ícones mudou (vai para a barra do topo principal).
pub const WM_BANDEJA_MUDOU: u32 = WM_APP + 8;

const MAGICO: i32 = 0x3475_3423;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x01;
const NIF_ICON: u32 = 0x02;
const NIF_TIP: u32 = 0x04;
const NIF_STATE: u32 = 0x08;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x01;
const NIN_SELECT: u32 = WM_USER;
const WM_CONTEXTMENU_: u32 = 0x007B;

/// NOTIFYICONDATA como o shell32 manda entre processos (handles em 32 bits).
#[repr(C)]
#[derive(Clone, Copy)]
struct Nid {
    cb_size: u32,
    hwnd: u32,
    id: u32,
    flags: u32,
    callback: u32,
    hicon: u32,
    dica: [u16; 128],
    estado: u32,
    mascara: u32,
    info: [u16; 256],
    versao: u32,
    titulo_info: [u16; 64],
    flags_info: u32,
    guid: GUID,
    icone_balao: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DadosBandeja {
    magico: i32,
    mensagem: u32,
    nid: Nid,
}

#[derive(Clone)]
pub struct Icone {
    pub hwnd: HWND,
    pub id: u32,
    pub guid: GUID,
    pub callback: u32,
    pub versao: u32,
    pub dica: String,
    /// Cópia nossa do ícone (o app pode destruir o dele).
    pub hicon: HICON,
    pub oculto: bool,
}

#[derive(Default)]
struct Estado {
    icones: Vec<Icone>,
}

thread_local! {
    static ESTADO: RefCell<Estado> = RefCell::new(Estado::default());
}

static AVISAR: AtomicIsize = AtomicIsize::new(0);
static NOSSA: AtomicIsize = AtomicIsize::new(0);
/// Estamos mandando o "TaskbarCreated" (para ignorar o nosso próprio).
pub static EMITINDO: AtomicBool = AtomicBool::new(false);

fn texto(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn h32(v: u32) -> HWND {
    HWND(v as usize as *mut _)
}

/// Cria a janela "Shell_TrayWnd" do Cais e pede aos programas que
/// reenviem seus ícones. `avisar` = janela que recebe WM_BANDEJA_MUDOU.
pub fn iniciar(avisar: HWND) -> bool {
    AVISAR.store(avisar.0 as isize, Ordering::Relaxed);
    unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(inst.0),
            lpszClassName: w!("Shell_TrayWnd"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let Ok(h) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("Shell_TrayWnd"),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            HINSTANCE(inst.0),
            None,
        ) else {
            return false;
        };
        NOSSA.store(h.0 as isize, Ordering::Relaxed);
        // Se o Cais rodar como administrador, o Windows bloquearia os
        // avisos de programas comuns: libera só os que a bandeja recebe.
        let _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
        let _ = ChangeWindowMessageFilterEx(h, RegisterWindowMessageW(w!("TaskbarCreated")), MSGFLT_ALLOW, None);
        reassumir();
        true
    }
}

/// Garante que a nossa janela é a primeira "Shell_TrayWnd" encontrada.
pub fn ficar_na_frente() {
    let h = HWND(NOSSA.load(Ordering::Relaxed) as *mut _);
    if h.0.is_null() {
        return;
    }
    unsafe {
        let _ = SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Fica na frente e pede (TaskbarCreated) para os programas mandarem os
/// ícones de novo.
pub fn reassumir() {
    ficar_na_frente();
    unsafe {
        let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        EMITINDO.store(true, Ordering::Relaxed);
        let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
        EMITINDO.store(false, Ordering::Relaxed);
    }
}

pub fn encerrar() {
    let h = HWND(NOSSA.swap(0, Ordering::Relaxed) as *mut _);
    if !h.0.is_null() {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
    ESTADO.with(|e| {
        if let Ok(mut e) = e.try_borrow_mut() {
            for i in e.icones.drain(..) {
                unsafe {
                    let _ = DestroyIcon(i.hicon);
                }
            }
        }
    });
}

/// Ícones visíveis agora.
pub fn icones() -> Vec<Icone> {
    ESTADO.with(|e| {
        e.try_borrow()
            .map(|e| e.icones.iter().filter(|i| !i.oculto && !i.hicon.0.is_null()).cloned().collect())
            .unwrap_or_default()
    })
}

/// Tira ícones de programas que fecharam sem avisar.
pub fn limpar_mortos() -> bool {
    let mut mudou = false;
    ESTADO.with(|e| {
        if let Ok(mut e) = e.try_borrow_mut() {
            e.icones.retain(|i| {
                let vivo = unsafe { IsWindow(i.hwnd).as_bool() };
                if !vivo {
                    unsafe {
                        let _ = DestroyIcon(i.hicon);
                    }
                    mudou = true;
                }
                vivo
            });
        }
    });
    mudou
}

/// Clique num ícone: manda ao programa as mesmas mensagens que a barra de
/// tarefas mandaria. (x, y) = ponto na tela, para o menu abrir ali.
pub fn clicar(i: &Icone, direito: bool, duplo: bool, x: i32, y: i32) {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(i.hwnd, Some(&mut pid));
        // Deixa o programa vir para a frente (abrir o menu ou a janela).
        let _ = AllowSetForegroundWindow(pid);
        let (desce, sobe) = if direito { (WM_RBUTTONDOWN, WM_RBUTTONUP) } else { (WM_LBUTTONDOWN, WM_LBUTTONUP) };
        let mandar = |m: u32| {
            let (wp, lp) = if i.versao >= 4 {
                (
                    WPARAM(((y as u16 as usize) << 16) | (x as u16 as usize)),
                    LPARAM(((i.id as isize & 0xffff) << 16) | m as isize),
                )
            } else {
                (WPARAM(i.id as usize), LPARAM(m as isize))
            };
            let _ = PostMessageW(i.hwnd, i.callback, wp, lp);
        };
        if duplo && !direito {
            mandar(WM_LBUTTONDBLCLK);
            mandar(WM_LBUTTONUP);
            return;
        }
        mandar(desce);
        mandar(sobe);
        if i.versao >= 4 {
            mandar(if direito { WM_CONTEXTMENU_ } else { NIN_SELECT });
        }
    }
}

// ---------------------------------------------------------------------------

/// Atualiza a nossa lista com uma mensagem da bandeja. Devolve true se mudou.
fn registrar(d: &DadosBandeja) -> bool {
    let n = &d.nid;
    let mesmo = |i: &Icone| {
        if n.flags & NIF_GUID != 0 && n.guid != GUID::zeroed() {
            i.guid == n.guid
        } else {
            i.hwnd == h32(n.hwnd) && i.id == n.id
        }
    };
    ESTADO.with(|e| {
        let Ok(mut e) = e.try_borrow_mut() else { return false };
        let pos = e.icones.iter().position(mesmo);
        match d.mensagem {
            NIM_ADD | NIM_MODIFY => {
                let k = match pos {
                    Some(k) => k,
                    None if d.mensagem == NIM_ADD => {
                        e.icones.push(Icone {
                            hwnd: h32(n.hwnd),
                            id: n.id,
                            guid: if n.flags & NIF_GUID != 0 { n.guid } else { GUID::zeroed() },
                            callback: 0,
                            versao: 0,
                            dica: String::new(),
                            hicon: HICON::default(),
                            oculto: false,
                        });
                        e.icones.len() - 1
                    }
                    None => return false,
                };
                let i = &mut e.icones[k];
                i.hwnd = h32(n.hwnd);
                i.id = n.id;
                if n.flags & NIF_MESSAGE != 0 {
                    i.callback = n.callback;
                }
                if n.flags & NIF_TIP != 0 {
                    i.dica = texto(&n.dica);
                }
                if n.flags & NIF_STATE != 0 && n.mascara & NIS_HIDDEN != 0 {
                    i.oculto = n.estado & NIS_HIDDEN != 0;
                }
                if n.flags & NIF_ICON != 0 {
                    let novo = unsafe { CopyIcon(HICON(n.hicon as usize as *mut _)) }.unwrap_or_default();
                    if !i.hicon.0.is_null() {
                        unsafe {
                            let _ = DestroyIcon(i.hicon);
                        }
                    }
                    i.hicon = novo;
                }
                true
            }
            NIM_DELETE => match pos {
                Some(k) => {
                    let i = e.icones.remove(k);
                    unsafe {
                        let _ = DestroyIcon(i.hicon);
                    }
                    true
                }
                None => false,
            },
            NIM_SETVERSION => {
                if let Some(k) = pos {
                    e.icones[k].versao = n.versao;
                }
                false
            }
            _ => false,
        }
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let explorer = || crate::barra::bandeja_do_explorer();
    match msg {
        WM_COPYDATA => {
            let cd = &*(lp.0 as *const COPYDATASTRUCT);
            // 1 = ícone da bandeja. Lê (cópia) e solta antes de repassar.
            if cd.dwData == 1 && cd.cbData as usize >= 8 && !cd.lpData.is_null() {
                let mut d: DadosBandeja = std::mem::zeroed();
                let n = (cd.cbData as usize).min(std::mem::size_of::<DadosBandeja>());
                std::ptr::copy_nonoverlapping(cd.lpData as *const u8, &mut d as *mut _ as *mut u8, n);
                if d.magico == MAGICO && registrar(&d) {
                    let a = HWND(AVISAR.load(Ordering::Relaxed) as *mut _);
                    let _ = PostMessageW(a, WM_BANDEJA_MUDOU, WPARAM(0), LPARAM(0));
                }
            }
            // Repassa tudo (bandeja, barras de aplicativo...) para o Explorer
            // e devolve a resposta dele.
            match explorer() {
                Some(e) => {
                    let mut r = 0usize;
                    let ok = SendMessageTimeoutW(e, WM_COPYDATA, wp, lp, SMTO_ABORTIFHUNG, 3000, Some(&mut r));
                    if ok.0 == 0 {
                        LRESULT(if cd.dwData == 1 { 1 } else { 0 })
                    } else {
                        LRESULT(r as isize)
                    }
                }
                None => LRESULT(if cd.dwData == 1 { 1 } else { 0 }),
            }
        }
        // Mensagens próprias da barra (WM_USER e registradas): também vão
        // para o Explorer, que é quem sabe responder.
        m if m == RegisterWindowMessageW(w!("TaskbarCreated")) => LRESULT(0),
        m if (WM_USER..0x8000).contains(&m) || m >= 0xC000 => match explorer() {
            Some(e) if e != hwnd => {
                let mut r = 0usize;
                let _ = SendMessageTimeoutW(e, m, wp, lp, SMTO_ABORTIFHUNG, 1000, Some(&mut r));
                LRESULT(r as isize)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        },
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
