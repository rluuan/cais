//! Criação de janelas sem moldura e efeito de vidro do Windows 11.

use windows::core::{w, Error, Result, PCSTR, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint,
    SetWindowRgn, HDC, HMONITOR, HRGN, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, RGN_OR,
};
use windows::Win32::UI::Shell::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASSE: PCWSTR = w!("CaisJanela");

pub fn registrar(wndproc: WNDPROC) -> Result<()> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: wndproc,
            hInstance: HINSTANCE(inst.0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASSE,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(Error::from_win32());
        }
    }
    Ok(())
}

/// Janela popup sem bitmap de redirecionamento: todo o conteúdo vem do
/// DirectComposition, então o Windows não guarda uma cópia extra em memória.
pub fn criar(estilo_ex: WINDOW_EX_STYLE) -> Result<HWND> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        CreateWindowExW(
            estilo_ex | WS_EX_NOREDIRECTIONBITMAP,
            CLASSE,
            w!("Cais"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            HINSTANCE(inst.0),
            None,
        )
    }
}

/// Libera o "host backdrop" (o blur do que está atrás da janela) para o
/// compositor e tira o arredondamento automático do Windows 11: quem dá
/// forma ao vidro é o próprio Cais.
pub fn preparar_vidro(hwnd: HWND) {
    unsafe {
        let sim = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_HOSTBACKDROPBRUSH,
            &sim as *const _ as _,
            std::mem::size_of::<BOOL>() as u32,
        );
        let canto = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &canto as *const _ as _,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// Área da janela que recebe o mouse (pixels). Fora dela o clique passa
/// para a janela de baixo. None = janela inteira.
pub fn definir_regiao(hwnd: HWND, r: Option<RECT>) {
    unsafe {
        match r {
            Some(r) => {
                let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
                SetWindowRgn(hwnd, rgn, true);
            }
            None => {
                SetWindowRgn(hwnd, HRGN::default(), true);
            }
        }
    }
}

/// Menus de contexto no tema do Cais (API não documentada do uxtheme, a
/// mesma que o Explorer usa; se não existir, os menus seguem o Windows).
pub fn menus_no_tema() {
    unsafe {
        let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        // 135 = SetPreferredAppMode, 136 = FlushMenuThemes
        if let Some(f) = GetProcAddress(ux, PCSTR(135 as *const u8)) {
            let definir: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            definir(crate::tema::modo_dos_menus());
        }
        if let Some(f) = GetProcAddress(ux, PCSTR(136 as *const u8)) {
            let aplicar: extern "system" fn() = std::mem::transmute(f);
            aplicar();
        }
    }
}

pub fn escala(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

pub fn area_de_trabalho() -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    rc
}

/// (retângulo do monitor principal, área de trabalho dele), em pixels.
pub fn monitor_principal() -> (RECT, RECT) {
    unsafe {
        let h = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(h, &mut mi);
        (mi.rcMonitor, mi.rcWork)
    }
}

/// (retângulo do monitor, área de trabalho dele), em pixels.
pub fn info_monitor(h: HMONITOR) -> (RECT, RECT) {
    unsafe {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(h, &mut mi).as_bool() {
            (mi.rcMonitor, mi.rcWork)
        } else {
            monitor_principal()
        }
    }
}

pub fn hmonitor_principal() -> HMONITOR {
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// Todos os monitores ligados, o principal primeiro.
pub fn monitores() -> Vec<HMONITOR> {
    unsafe extern "system" fn juntar(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<HMONITOR>);
        v.push(h);
        BOOL(1)
    }
    let mut v: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(juntar), LPARAM(&mut v as *mut _ as isize));
    }
    let p = hmonitor_principal();
    v.sort_by_key(|&h| h != p);
    v
}

/// Monitor onde o mouse está agora.
pub fn monitor_do_cursor() -> (RECT, RECT) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        info_monitor(MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY))
    }
}

fn dados_appbar(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    }
}

/// Registra o dock como "barra de aplicativo": o Windows passa a avisar
/// (pela mensagem `msg`) quando um app entra em tela cheia, e deixa o dock
/// reservar espaço na tela.
pub fn registrar_appbar(hwnd: HWND, msg: u32) {
    let mut d = dados_appbar(hwnd);
    d.uCallbackMessage = msg;
    unsafe {
        SHAppBarMessage(ABM_NEW, &mut d);
    }
}

pub fn remover_appbar(hwnd: HWND) {
    let mut d = dados_appbar(hwnd);
    unsafe {
        SHAppBarMessage(ABM_REMOVE, &mut d);
    }
}

/// Reserva `altura` px na base (`topo` = false) ou no topo do monitor
/// (0 = não reserva nada) e devolve o retângulo reservado: o Windows o
/// empurra para não ficar em cima de outras barras (como a de tarefas).
/// Só avisa o Windows quando a reserva muda de verdade.
pub fn reservar_appbar(
    hwnd: HWND,
    mon: RECT,
    topo: bool,
    altura: i32,
    ultima: &mut Option<RECT>,
) -> RECT {
    let mut d = dados_appbar(hwnd);
    d.uEdge = if topo { ABE_TOP } else { ABE_BOTTOM };
    d.rc = if topo {
        RECT {
            left: mon.left,
            top: mon.top,
            right: mon.right,
            bottom: mon.top + altura,
        }
    } else {
        RECT {
            left: mon.left,
            top: mon.bottom - altura,
            right: mon.right,
            bottom: mon.bottom,
        }
    };
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &mut d);
        if topo {
            d.rc.bottom = d.rc.top + altura;
        } else {
            d.rc.top = d.rc.bottom - altura;
        }
        if *ultima != Some(d.rc) {
            SHAppBarMessage(ABM_SETPOS, &mut d);
            *ultima = Some(d.rc);
            crate::encaixe::agendar();
        }
    }
    d.rc
}

/// Reserva `largura` px na borda esquerda (0 = devolve o espaço).
pub fn reservar_appbar_esquerda(hwnd: HWND, mon: RECT, largura: i32, ultima: &mut Option<RECT>) -> RECT {
    let mut d = dados_appbar(hwnd);
    d.uEdge = ABE_LEFT;
    d.rc = RECT {
        left: mon.left,
        top: mon.top,
        right: mon.left + largura,
        bottom: mon.bottom,
    };
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &mut d);
        d.rc.right = d.rc.left + largura;
        if *ultima != Some(d.rc) {
            SHAppBarMessage(ABM_SETPOS, &mut d);
            *ultima = Some(d.rc);
            crate::encaixe::agendar();
        }
    }
    d.rc
}

/// Reserva `largura` px na borda esquerda ou direita (dock de lado).
pub fn reservar_appbar_lado(hwnd: HWND, mon: RECT, direita: bool, largura: i32, ultima: &mut Option<RECT>) -> RECT {
    let mut d = dados_appbar(hwnd);
    d.uEdge = if direita { ABE_RIGHT } else { ABE_LEFT };
    d.rc = if direita {
        RECT { left: mon.right - largura, top: mon.top, right: mon.right, bottom: mon.bottom }
    } else {
        RECT { left: mon.left, top: mon.top, right: mon.left + largura, bottom: mon.bottom }
    };
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &mut d);
        if direita {
            d.rc.left = d.rc.right - largura;
        } else {
            d.rc.right = d.rc.left + largura;
        }
        if *ultima != Some(d.rc) {
            SHAppBarMessage(ABM_SETPOS, &mut d);
            *ultima = Some(d.rc);
            crate::encaixe::agendar();
        }
    }
    d.rc
}

/// "Efeitos de animação" do Windows estão ligados? (Acessibilidade >
/// Efeitos visuais.) Desligados, o Cais reduz o movimento.
pub fn animacoes_do_windows() -> bool {
    let mut ligado = BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut ligado as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    ligado.as_bool()
}

/// Alto contraste do Windows está ligado?
pub fn alto_contraste() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    unsafe {
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
        {
            return false;
        }
    }
    hc.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0
}

/// Região de clique feita de vários retângulos (vazia = nada recebe clique).
pub fn definir_regiao_varias(hwnd: HWND, rs: &[RECT]) {
    unsafe {
        let total = CreateRectRgn(0, 0, 0, 0);
        for r in rs {
            let um = CreateRectRgn(r.left, r.top, r.right, r.bottom);
            CombineRgn(total, total, um, RGN_OR);
            let _ = DeleteObject(um);
        }
        SetWindowRgn(hwnd, total, true);
    }
}

pub fn tamanho_cliente(hwnd: HWND) -> (u32, u32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    (
        (rc.right - rc.left).max(1) as u32,
        (rc.bottom - rc.top).max(1) as u32,
    )
}

pub fn posicionar(hwnd: HWND, x: i32, y: i32, l: i32, a: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, l, a, SWP_NOACTIVATE);
    }
}
