//! Teste (cais.exe --testar-3d): verifica se este Windows deixa criar uma
//! miniatura ao vivo como camada do compositor, para incliná-la em 3D de
//! verdade. Mostra por 4 s uma janela que não pega mouse nem teclado.

use windows::core::{w, Interface, Result, HRESULT, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Liga o 3D na miniatura (flag não documentada).
pub const DWM_TNP_ENABLE3D: u32 = 0x0400_0000;

pub type CriarVisual = unsafe extern "system" fn(
    HWND,
    HWND,
    u32,
    *const DWM_THUMBNAIL_PROPERTIES,
    *mut core::ffi::c_void,
    *mut *mut core::ffi::c_void,
    *mut isize,
) -> HRESULT;

/// DwmpCreateSharedThumbnailVisual (ordinal 147 do dwmapi.dll).
pub fn funcao() -> Option<CriarVisual> {
    unsafe {
        let dwm = LoadLibraryW(w!("dwmapi.dll")).ok()?;
        let f = GetProcAddress(dwm, PCSTR(147 as *const u8))?;
        Some(std::mem::transmute::<_, CriarVisual>(f))
    }
}

pub fn mult(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0; 16];
    for i in 0..4 {
        for j in 0..4 {
            r[i * 4 + j] = (0..4).map(|k| a[i * 4 + k] * b[k * 4 + j]).sum();
        }
    }
    r
}

/// Gira em torno do eixo vertical que passa por (cx, cy) e aplica
/// perspectiva com a câmera a `d` px. Convenção de vetor-linha (D2D).
pub fn inclinar(cx: f32, cy: f32, graus: f32, d: f32) -> [f32; 16] {
    let (s, c) = graus.to_radians().sin_cos();
    let ida = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -cx, -cy, 0.0, 1.0];
    let gira = [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0];
    let persp = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -1.0 / d, 0.0, 0.0, 0.0, 1.0];
    let volta = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, cx, cy, 0.0, 1.0];
    mult(&mult(&mult(&ida, &gira), &persp), &volta)
}

pub fn matriz(m: [f32; 16]) -> D2D_MATRIX_4X4_F {
    D2D_MATRIX_4X4_F { Anonymous: D2D_MATRIX_4X4_F_0 { m } }
}

pub fn rodar() -> Result<()> {
    let Some(criar) = funcao() else {
        println!("3d: dwmapi sem a função 147");
        return Ok(());
    };
    // Alguma janela de app (de preferência minimizada).
    let janelas = crate::apps::enumerar_janelas();
    let fonte = janelas
        .iter()
        .find(|(h, _)| unsafe { IsIconic(*h).as_bool() })
        .or(janelas.first())
        .map(|(h, _)| *h);
    let Some(fonte) = fonte else {
        println!("3d: nenhuma janela para testar");
        return Ok(());
    };
    unsafe {
        let hwnd = crate::janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT)?;
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 300, 200, 700, 500, SWP_NOACTIVATE);
        let mut d3d = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        )?;
        let dxgi: IDXGIDevice = d3d.unwrap().cast()?;
        let dev: IDCompositionDesktopDevice = DCompositionCreateDevice3(&dxgi)?;
        let alvo = dev.CreateTargetForHwnd(hwnd, true)?;
        let raiz = dev.CreateVisual()?;
        alvo.SetRoot(&raiz)?;
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY | DWM_TNP_ENABLE3D,
            rcDestination: RECT { left: 0, top: 0, right: 480, bottom: 300 },
            rcSource: RECT::default(),
            opacity: 255,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        let mut visual: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut miniatura: isize = 0;
        let hr = criar(hwnd, fonte, 2, &props, dev.as_raw(), &mut visual, &mut miniatura);
        println!("3d: hr={:#010x} visual={} miniatura={}", hr.0, !visual.is_null(), miniatura);
        if hr.is_err() || visual.is_null() {
            let _ = DestroyWindow(hwnd);
            return Ok(());
        }
        let v = IDCompositionVisual2::from_raw(visual);
        raiz.AddVisual(&v, true, None)?;
        if let Ok(v3) = v.cast::<IDCompositionVisual3>() {
            v3.SetOffsetX2(100.0)?;
            v3.SetOffsetY2(100.0)?;
            v3.SetTransform2(&matriz(inclinar(0.0, 150.0, 28.0, 900.0)))?;
            println!("3d: transformação 3D aplicada");
        } else {
            println!("3d: sem IDCompositionVisual3");
        }
        dev.Commit()?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let fim = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let mut msg = MSG::default();
        while std::time::Instant::now() < fim {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let _ = DwmUnregisterThumbnail(miniatura);
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}
